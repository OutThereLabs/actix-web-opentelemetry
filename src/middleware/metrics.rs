//! # Metrics Middleware

use actix_web::dev;
use futures_util::future::{self, FutureExt as _, LocalBoxFuture};
use opentelemetry::{
    metrics::{Counter, Meter, ValueRecorder},
    Key,
};
use std::{sync::Arc, time::SystemTime};

use crate::RouteFormatter;

const ROUTE_KEY: Key = Key::from_static_str("route");
const METHOD_KEY: Key = Key::from_static_str("method");
const STATUS_KEY: Key = Key::from_static_str("status");

#[derive(Clone, Debug)]
struct Metrics {
    http_requests_total: Counter<u64>,
    http_requests_duration_seconds: ValueRecorder<f64>,
}

impl Metrics {
    /// Create a new [`RequestMetrics`]
    fn new(meter: Meter) -> Self {
        let http_requests_total = meter
            .u64_counter("http_request_total")
            .with_description("HTTP requests per route")
            .init();

        let http_requests_duration_seconds = meter
            .f64_value_recorder("http_request_duration_seconds")
            .with_description("HTTP request duration per route")
            // TODO: https://github.com/open-telemetry/opentelemetry-rust/issues/276
            // .with_unit(Unit::new("seconds"))
            .init();

        Metrics {
            http_requests_total,
            http_requests_duration_seconds,
        }
    }
}

/// Builder for [RequestMetrics]
#[derive(Clone, Debug, Default)]
pub struct RequestMetricsBuilder {
    route_formatter: Option<Arc<dyn RouteFormatter + Send + Sync + 'static>>,
}

impl RequestMetricsBuilder {
    /// Create a new `RequestMetricsBuilder`
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a route formatter to customize metrics match patterns
    pub fn with_route_formatter<R>(mut self, route_formatter: R) -> Self
    where
        R: RouteFormatter + Send + Sync + 'static,
    {
        self.route_formatter = Some(Arc::new(route_formatter));
        self
    }

    /// Build the `RequestMetrics` middleware
    pub fn build(self, meter: Meter) -> RequestMetrics {
        RequestMetrics {
            route_formatter: self.route_formatter,
            metrics: Arc::new(Metrics::new(meter)),
        }
    }
}

/// Request metrics tracking
///
/// # Examples
///
/// ```no_run
/// use actix_web::{dev, http, web, App, HttpRequest, HttpServer};
/// use actix_web_opentelemetry::{PrometheusMetricsHandler, RequestMetricsBuilder, RequestTracing};
/// use opentelemetry::global;
///
/// # async fn start_server() -> std::io::Result<()> {
/// let meter = global::meter("actix_web");
///
/// // Request metrics middleware
/// let request_metrics = RequestMetricsBuilder::new().build(meter);
///
/// #[cfg(feature = "metrics-prometheus")]
/// let exporter = opentelemetry_prometheus::exporter().init();
///
/// // Run actix server, metrics are now available at http://localhost:8080/metrics
/// HttpServer::new(move || {
///         let app = App::new().wrap(RequestTracing::new()).wrap(request_metrics.clone());
///
///         #[cfg(feature = "metrics-prometheus")]
///         let app = app.route("/metrics", web::get().to(PrometheusMetricsHandler::new(exporter.clone())));
///
///         app
///     })
///     .bind("localhost:8080")?
///     .run()
///     .await
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct RequestMetrics {
    route_formatter: Option<Arc<dyn RouteFormatter + Send + Sync + 'static>>,
    metrics: Arc<Metrics>,
}

impl<S, B> dev::Transform<S, dev::ServiceRequest> for RequestMetrics
where
    S: dev::Service<
        dev::ServiceRequest,
        Response = dev::ServiceResponse<B>,
        Error = actix_web::Error,
    >,
    S::Future: 'static,
    B: 'static,
{
    type Response = dev::ServiceResponse<B>;
    type Error = actix_web::Error;
    type Transform = RequestMetricsMiddleware<S>;
    type InitError = ();
    type Future = future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let service = RequestMetricsMiddleware {
            service,
            metrics: self.metrics.clone(),
            route_formatter: self.route_formatter.clone(),
        };

        future::ok(service)
    }
}

/// Request metrics middleware
#[allow(missing_debug_implementations)]
pub struct RequestMetricsMiddleware<S> {
    service: S,
    metrics: Arc<Metrics>,
    route_formatter: Option<Arc<dyn RouteFormatter + Send + Sync + 'static>>,
}

impl<S, B> dev::Service<dev::ServiceRequest> for RequestMetricsMiddleware<S>
where
    S: dev::Service<
        dev::ServiceRequest,
        Response = dev::ServiceResponse<B>,
        Error = actix_web::Error,
    >,
    S::Future: 'static,
    B: 'static,
{
    type Response = dev::ServiceResponse<B>;
    type Error = actix_web::Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    dev::forward_ready!(service);

    fn call(&self, req: dev::ServiceRequest) -> Self::Future {
        let timer = SystemTime::now();
        let request_metrics = self.metrics.clone();
        let mut route = req.match_pattern().unwrap_or_else(|| "default".to_string());
        if let Some(formatter) = &self.route_formatter {
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

#[cfg(feature = "metrics-prometheus")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics-prometheus")))]
pub(crate) mod prometheus {
    use actix_web::{dev, http::StatusCode};
    use futures_util::future::{self, LocalBoxFuture};
    use opentelemetry::{global, metrics::MetricsError};
    use opentelemetry_prometheus::PrometheusExporter;
    use prometheus::{Encoder, TextEncoder};

    /// Prometheus request metrics service
    #[derive(Clone, Debug)]
    pub struct PrometheusMetricsHandler {
        prometheus_exporter: PrometheusExporter,
    }

    impl PrometheusMetricsHandler {
        /// Build a route to serve Prometheus metrics
        pub fn new(exporter: PrometheusExporter) -> Self {
            Self {
                prometheus_exporter: exporter,
            }
        }
    }

    impl PrometheusMetricsHandler {
        fn metrics(&self) -> String {
            let encoder = TextEncoder::new();
            let metric_families = self.prometheus_exporter.registry().gather();
            let mut buf = Vec::new();
            if let Err(err) = encoder.encode(&metric_families[..], &mut buf) {
                global::handle_error(MetricsError::Other(err.to_string()));
            }

            String::from_utf8(buf).unwrap_or_default()
        }
    }

    impl dev::Handler<actix_web::HttpRequest> for PrometheusMetricsHandler {
        type Output = Result<actix_web::HttpResponse<String>, actix_web::error::Error>;
        type Future = LocalBoxFuture<'static, Self::Output>;

        fn call(&self, _req: actix_web::HttpRequest) -> Self::Future {
            Box::pin(future::ok(actix_web::HttpResponse::with_body(
                StatusCode::OK,
                self.metrics(),
            )))
        }
    }
}
