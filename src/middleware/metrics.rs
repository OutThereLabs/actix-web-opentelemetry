//! # Metrics Middleware

use actix_web::dev;
use futures_util::future::{self, FutureExt as _, LocalBoxFuture};
use opentelemetry::metrics::{Counter, Meter, Unit, UpDownCounter, ValueRecorder};
use std::{sync::Arc, time::SystemTime};

use crate::util::trace_attributes_from_request;
use crate::RouteFormatter;

// Follows the experimental semantic conventions for HTTP metrics:
// https://github.com/open-telemetry/opentelemetry-specification/blob/main/specification/metrics/semantic_conventions/http-metrics.md
use opentelemetry_semantic_conventions::trace::HTTP_STATUS_CODE;
const HTTP_SERVER_ACTIVE_REQUESTS: &str = "http.server.active_requests";
const HTTP_SERVER_TOTAL_REQUESTS: &str = "http.server.total_requests";
const HTTP_SERVER_DURATION: &str = "http.server.duration";

#[derive(Clone, Debug)]
struct Metrics {
    http_server_active_requests: UpDownCounter<i64>,
    http_server_total_requests: Counter<u64>,
    http_server_duration: ValueRecorder<f64>,
}

impl Metrics {
    /// Create a new [`RequestMetrics`]
    fn new(meter: Meter) -> Self {
        let http_server_active_requests = meter
            .i64_up_down_counter(HTTP_SERVER_ACTIVE_REQUESTS)
            .with_description("HTTP concurrent in-flight requests per route")
            .init();

        let http_server_total_requests = meter
            .u64_counter(HTTP_SERVER_TOTAL_REQUESTS)
            .with_description("HTTP requests per route")
            .init();

        let http_server_duration = meter
            .f64_value_recorder(HTTP_SERVER_DURATION)
            .with_description("HTTP inbound request duration per route")
            .with_unit(Unit::new("ms"))
            .init();

        Metrics {
            http_server_active_requests,
            http_server_total_requests,
            http_server_duration,
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
/// use actix_web_opentelemetry::{
///     PrometheusMetricsHandler,
///     RequestMetricsBuilder,
///     RequestTracing,
/// };
/// use opentelemetry::global;
///
/// # #[cfg(feature = "metrics-prometheus")]
/// #[actix_web::main]
/// async fn main() -> std::io::Result<()> {
///     // Request metrics middleware
///     let meter = global::meter("actix_web");
///     let request_metrics = RequestMetricsBuilder::new().build(meter);
///
///     // Prometheus request metrics handler
///     let exporter = opentelemetry_prometheus::exporter().init();
///     let metrics_handler = PrometheusMetricsHandler::new(exporter);
///
///     // Run actix server, metrics are now available at http://localhost:8080/metrics
///     HttpServer::new(move || {
///         App::new()
///             .wrap(RequestTracing::new())
///             .wrap(request_metrics.clone())
///             .route("/metrics", web::get().to(metrics_handler.clone()))
///     })
///     .bind("localhost:8080")?
///     .run()
///     .await
/// }
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

        let mut http_route = req.match_pattern().unwrap_or_else(|| "default".to_string());
        if let Some(formatter) = &self.route_formatter {
            http_route = formatter.format(&http_route);
        }

        let mut attributes = trace_attributes_from_request(&req, &http_route);

        let http_server_active_requests =
            self.metrics.http_server_active_requests.bind(&attributes);
        http_server_active_requests.add(1);

        let request_metrics = self.metrics.clone();
        Box::pin(self.service.call(req).map(move |res| {
            http_server_active_requests.add(-1);

            // Ignore actix errors for metrics
            if let Ok(res) = res {
                attributes.push(HTTP_STATUS_CODE.string(res.status().as_str().to_owned()));

                request_metrics
                    .http_server_total_requests
                    .add(1, &attributes);

                request_metrics.http_server_duration.record(
                    timer
                        .elapsed()
                        .map(|t| t.as_secs_f64() * 1000.0)
                        .unwrap_or_default(),
                    &attributes,
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
