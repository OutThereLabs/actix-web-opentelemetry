//! # Metrics Middleware
use crate::{PassThroughFormatter, RouteFormatter};
use actix_web::dev;
use futures::{
    future::{self, FutureExt},
    Future,
};
use opentelemetry::{
    api::{self, Counter, Measure, Meter, MetricOptions},
    exporter::metrics::prometheus::{self, Encoder},
};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::SystemTime;

/// Request metrics tracking
#[derive(Debug)]
pub struct RequestMetrics<M, R, F>
where
    M: api::Meter,
    M::I64Counter: Clone,
    M::F64Measure: Clone,
    R: RouteFormatter,
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    sdk: Arc<M>,
    route_formatter: R,
    should_render_metrics: Option<F>,
    http_requests_total: M::I64Counter,
    http_requests_duration_seconds: M::F64Measure,
}

impl<M, R, F> Clone for RequestMetrics<M, R, F>
where
    M: api::Meter,
    M::I64Counter: Clone,
    M::F64Measure: Clone,
    R: RouteFormatter + Clone,
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    fn clone(&self) -> Self {
        RequestMetrics {
            sdk: self.sdk.clone(),
            route_formatter: self.route_formatter.clone(),
            should_render_metrics: self.should_render_metrics.clone(),
            http_requests_total: self.http_requests_total.clone(),
            http_requests_duration_seconds: self.http_requests_duration_seconds.clone(),
        }
    }
}

impl<F> Default for RequestMetrics<api::NoopMeter, PassThroughFormatter, F>
where
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    fn default() -> Self {
        let sdk = Arc::new(api::NoopMeter {});
        let http_requests_total = sdk.new_i64_counter("", MetricOptions::default());
        let http_requests_duration_seconds = sdk.new_f64_measure("", MetricOptions::default());
        RequestMetrics {
            sdk,
            route_formatter: PassThroughFormatter,
            should_render_metrics: None,
            http_requests_total,
            http_requests_duration_seconds,
        }
    }
}

impl<M, R, F> RequestMetrics<M, R, F>
where
    M: api::Meter,
    M::I64Counter: Clone,
    M::F64Measure: Clone,
    R: RouteFormatter,
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    /// Create new `RequestMetrics`
    pub fn new(sdk: M, route_formatter: R, should_render_metrics: Option<F>) -> Self {
        let standard_keys = vec![
            api::Key::new("route"),
            api::Key::new("method"),
            api::Key::new("status"),
        ];
        let http_requests_total = sdk.new_i64_counter(
            "http_requests_total",
            MetricOptions::default()
                .with_description("HTTP requests per route")
                .with_keys(standard_keys.clone()),
        );
        let http_requests_duration_seconds = sdk.new_f64_measure(
            "http_requests_duration",
            MetricOptions::default()
                .with_description("HTTP request duration per route")
                .with_unit(api::Unit::new("seconds"))
                .with_keys(standard_keys),
        );
        RequestMetrics {
            sdk: Arc::new(sdk),
            route_formatter,
            should_render_metrics,
            http_requests_total,
            http_requests_duration_seconds,
        }
    }

    fn metrics(&self) -> String {
        let mut buffer = vec![];
        prometheus::TextEncoder::new()
            .encode(&prometheus::gather(), &mut buffer)
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }
}

impl<S, B, M, R, F> dev::Transform<S> for RequestMetrics<M, R, F>
where
    S: dev::Service<
        Request = dev::ServiceRequest,
        Response = dev::ServiceResponse<B>,
        Error = actix_web::Error,
    >,
    S::Future: 'static,
    B: 'static,
    M: api::Meter + 'static,
    M::I64Counter: Clone,
    M::F64Measure: Clone,
    R: RouteFormatter + Clone + 'static,
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone + 'static,
{
    type Request = dev::ServiceRequest;
    type Response = dev::ServiceResponse<B>;
    type Error = actix_web::Error;
    type Transform = RequestMetricsMiddleware<S, M, R, F>;
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
pub struct RequestMetricsMiddleware<S, M, R, F>
where
    M: api::Meter,
    M::I64Counter: Clone,
    M::F64Measure: Clone,
    R: RouteFormatter,
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone,
{
    service: S,
    inner: Arc<RequestMetrics<M, R, F>>,
}

impl<S, B, M, R, F> dev::Service for RequestMetricsMiddleware<S, M, R, F>
where
    S: dev::Service<
        Request = dev::ServiceRequest,
        Response = dev::ServiceResponse<B>,
        Error = actix_web::Error,
    >,
    S::Future: 'static,
    B: 'static,
    M: api::Meter + 'static,
    M::I64Counter: Clone,
    M::F64Measure: Clone,
    R: RouteFormatter + 'static,
    F: Fn(&dev::ServiceRequest) -> bool + Send + Clone + 'static,
{
    type Request = dev::ServiceRequest;
    type Response = dev::ServiceResponse<B>;
    type Error = actix_web::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: dev::ServiceRequest) -> Self::Future {
        if self
            .inner
            .should_render_metrics
            .as_ref()
            .map(|f| f(&req))
            .unwrap_or(false)
        {
            Box::pin(future::ok(
                req.into_response(
                    actix_web::HttpResponse::Ok()
                        .body(dev::Body::from_message(self.inner.metrics()))
                        .into_body(),
                ),
            ))
        } else {
            let timer = SystemTime::now();
            let request_metrics = self.inner.clone();
            let route = request_metrics.route_formatter.format(req.path());
            let method = req.method().as_str().to_string();

            Box::pin(self.service.call(req).map(move |res| {
                // Ignore actix errors for metrics
                if let Ok(res) = res {
                    let standard_labels = request_metrics.sdk.labels(vec![
                        api::KeyValue::new("route", route.as_str()),
                        api::KeyValue::new("method", method.as_str()),
                        api::KeyValue::new("status", api::Value::U64(res.status().as_u16() as u64)),
                    ]);
                    request_metrics.http_requests_total.add(1, &standard_labels);
                    request_metrics.http_requests_duration_seconds.record(
                        timer.elapsed().map(|t| t.as_secs_f64()).unwrap_or(0.0),
                        &standard_labels,
                    );

                    Ok(res)
                } else {
                    res
                }
            }))
        }
    }
}
