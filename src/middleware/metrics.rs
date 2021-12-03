//! # Metrics Middleware
use crate::RouteFormatter;
use actix_web::{dev, http::StatusCode};
use futures::{
    future::{self, FutureExt},
    Future,
};
use opentelemetry::{
    global,
    metrics::{
        noop::NoopMeterProvider, Counter, Meter, MeterProvider, MetricsError, ValueRecorder,
    },
    Key,
};
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::{Encoder, TextEncoder};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::SystemTime;

/// Request metrics tracking
///
/// # Examples
///
/// ```no_run
/// use actix_web::{dev, http, web, App, HttpRequest, HttpServer};
/// use actix_web_opentelemetry::RequestMetrics;
/// use opentelemetry::global;
///
/// # async fn start_server() -> std::io::Result<()> {
/// let exporter = opentelemetry_prometheus::exporter().init();
/// let meter = global::meter("actix_web");
///
/// // Optional predicate to determine which requests render the prometheus metrics
/// let metrics_route = |req: &dev::ServiceRequest| {
///     req.path() == "/metrics" && req.method() == http::Method::GET
/// };
///
/// // Request metrics middleware
/// let request_metrics = RequestMetrics::new(meter, Some(metrics_route), Some(exporter));
///
/// // Run actix server, metrics are now available at http://localhost:8080/metrics
/// HttpServer::new(move || App::new().wrap(request_metrics.clone()))
///     .bind("localhost:8080")?
///     .run()
///     .await
/// # }
/// ```
#[derive(Debug)]
pub struct RequestMetrics<F>
where
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    exporter: PrometheusExporter,
    route_formatter: Option<Arc<dyn RouteFormatter + Send + Sync + 'static>>,
    should_render_metrics: Option<F>,
    http_requests_total: Counter<u64>,
    http_requests_duration_seconds: ValueRecorder<f64>,
}

impl<F> Clone for RequestMetrics<F>
where
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    fn clone(&self) -> Self {
        RequestMetrics {
            exporter: self.exporter.clone(),
            route_formatter: self.route_formatter.clone(),
            should_render_metrics: self.should_render_metrics.clone(),
            http_requests_total: self.http_requests_total.clone(),
            http_requests_duration_seconds: self.http_requests_duration_seconds.clone(),
        }
    }
}

impl<F> Default for RequestMetrics<F>
where
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    fn default() -> Self {
        let provider = NoopMeterProvider::new();
        let meter = provider.meter("noop", None);
        RequestMetrics::new(meter, None, None)
    }
}

const ROUTE_KEY: Key = Key::from_static_str("route");
const METHOD_KEY: Key = Key::from_static_str("method");
const STATUS_KEY: Key = Key::from_static_str("status");

impl<F> RequestMetrics<F>
where
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    /// Create a new [`RequestMetrics`]
    pub fn new(
        meter: Meter,
        should_render_metrics: Option<F>,
        exporter: Option<PrometheusExporter>,
    ) -> Self {
        let exporter = exporter.unwrap_or_else(|| opentelemetry_prometheus::exporter().init());
        let http_requests_total = meter
            .u64_counter("http_requests_total")
            .with_description("HTTP requests per route")
            .init();

        let http_requests_duration_seconds = meter
            .f64_value_recorder("http_requests_duration")
            .with_description("HTTP request duration per route")
            // TODO: https://github.com/open-telemetry/opentelemetry-rust/issues/276
            // .with_unit(Unit::new("seconds"))
            .init();

        RequestMetrics {
            exporter,
            route_formatter: None,
            should_render_metrics,
            http_requests_total,
            http_requests_duration_seconds,
        }
    }

    /// Add a route formatter to customize metrics match patterns
    pub fn with_route_formatter<R>(mut self, route_formatter: R) -> Self
    where
        R: RouteFormatter + Send + Sync + 'static,
    {
        self.route_formatter = Some(Arc::new(route_formatter));
        self
    }

    fn metrics(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.exporter.registry().gather();
        let mut buf = Vec::new();
        if let Err(err) = encoder.encode(&metric_families[..], &mut buf) {
            global::handle_error(MetricsError::Other(err.to_string()));
        }

        String::from_utf8(buf).unwrap_or_default()
    }
}

impl<S, F> dev::Transform<S, dev::ServiceRequest> for RequestMetrics<F>
where
    S: dev::Service<
        dev::ServiceRequest,
        Response = dev::ServiceResponse,
        Error = actix_web::Error,
    >,
    S::Future: 'static,
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone + 'static,
{
    type Response = dev::ServiceResponse;
    type Error = actix_web::Error;
    type Transform = RequestMetricsMiddleware<S, F>;
    type InitError = ();
    type Future = future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        future::ok(RequestMetricsMiddleware {
            service,
            inner: Arc::new((*self).clone()),
        })
    }
}

/// Request metrics middleware
#[allow(missing_debug_implementations)]
pub struct RequestMetricsMiddleware<S, F>
where
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    service: S,
    inner: Arc<RequestMetrics<F>>,
}

impl<S, F> dev::Service<dev::ServiceRequest> for RequestMetricsMiddleware<S, F>
where
    S: dev::Service<
        dev::ServiceRequest,
        Response = dev::ServiceResponse,
        Error = actix_web::Error,
    >,
    S::Future: 'static,
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone + 'static,
{
    type Response = dev::ServiceResponse;
    type Error = actix_web::Error;
    #[allow(clippy::type_complexity)]
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: dev::ServiceRequest) -> Self::Future {
        if self
            .inner
            .should_render_metrics
            .as_ref()
            .map(|f| f(&req))
            .unwrap_or(false)
        {
            Box::pin(future::ok(
                req.into_response(
                    actix_web::HttpResponse::with_body(StatusCode::OK, dev::AnyBody::new_boxed(self.inner.metrics()))
                ),
            ))
        } else {
            let timer = SystemTime::now();
            let request_metrics = self.inner.clone();
            let mut route = req.match_pattern().unwrap_or_else(|| "default".to_string());
            if let Some(formatter) = &self.inner.route_formatter {
                route = formatter.format(&route);
            }
            let method = req.method().as_str().to_string();

            Box::pin(self.service.call(req).map(move |res| {
                // Ignore actix errors for metrics
                if let Ok(res) = res {
                    let labels = vec![
                        ROUTE_KEY.string(route),
                        METHOD_KEY.string(method),
                        STATUS_KEY.i64(res.status().as_u16() as i64),
                    ];
                    request_metrics.http_requests_total.add(1, &labels);
                    request_metrics.http_requests_duration_seconds.record(
                        timer.elapsed().map(|t| t.as_secs_f64()).unwrap_or_default(),
                        &labels,
                    );

                    Ok(res)
                } else {
                    res
                }
            }))
        }
    }
}
