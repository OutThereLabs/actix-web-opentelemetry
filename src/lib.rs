//! [OpenTelemetry] integration for [Actix Web].
//!
//! This crate allows you to easily instrument client and server requests.
//!
//! * Server requests can be traced by using the [`RequestTracing`] middleware.
//!
//! The `awc` feature allows you to instrument client requests made by the [awc] crate.
//!
//! * Client requests can be traced by using the [`ClientExt::trace_request`] method.
//!
//! The `metrics` feature allows you to expose request metrics to [Prometheus].
//!
//! * Metrics can be tracked using the [`RequestMetrics`] middleware.
//!
//! [OpenTelemetry]: https://opentelemetry.io
//! [Actix Web]: https://actix.rs
//! [awc]: https://docs.rs/awc
//! [Prometheus]: https://prometheus.io
//!
//! ### Client Request Examples:
//!
//! Note: this requires the `awc` feature to be enabled.
//!
//! ```no_run
//! # #[cfg(feature="awc")]
//! # {
//! use awc::{Client, error::SendRequestError};
//! use actix_web_opentelemetry::ClientExt;
//!
//! async fn execute_request(client: &Client) -> Result<(), SendRequestError> {
//!     let res = client
//!         .get("http://localhost:8080")
//!         // Add `trace_request` before `send` to any awc request to add instrumentation
//!         .trace_request()
//!         .send()
//!         .await?;
//!
//!     println!("Response: {:?}", res);
//!     Ok(())
//! }
//! # }
//! ```
//!
//! ### Server middleware examples:
//!
//! Tracing and metrics middleware can be used together or independently.
//!
//! Tracing server example:
//!
//! ```no_run
//! use actix_web::{web, App, HttpServer};
//! use actix_web_opentelemetry::RequestTracing;
//!
//! async fn index() -> &'static str {
//!     "Hello world!"
//! }
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     // Install an OpenTelemetry trace pipeline.
//!     // Swap for https://docs.rs/opentelemetry-jaeger or other compatible
//!     // exporter to send trace information to your collector.
//!     opentelemetry::sdk::export::trace::stdout::new_pipeline().install_simple();
//!
//!     HttpServer::new(|| {
//!         App::new()
//!             .wrap(RequestTracing::new())
//!             .service(web::resource("/").to(index))
//!     })
//!     .bind("127.0.0.1:8080")?
//!     .run()
//!     .await
//! }
//! ```
//!
//! Request metrics middleware (requires the `metrics` feature):
//!
//! ```no_run
//! use actix_web::{dev, http, web, App, HttpRequest, HttpServer};
//! #[cfg(feature = "metrics-prometheus")]
//! use actix_web_opentelemetry::{PrometheusMetricsHandler, RequestMetricsBuilder, RequestTracing};
//! use opentelemetry::global;
//!
//! #[cfg(feature = "metrics-prometheus")]
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     let exporter = opentelemetry_prometheus::exporter().init();
//!     let meter = global::meter("actix_web");
//!
//!     // Request metrics middleware
//!     let request_metrics = RequestMetricsBuilder::new().build(meter);
//!
//!     // Run actix server, metrics are now available at http://localhost:8080/metrics
//!     HttpServer::new(move || {
//!         App::new()
//!             .wrap(RequestTracing::new())
//!             .wrap(request_metrics.clone())
//!             .route("/metrics", web::get().to(PrometheusMetricsHandler::new(exporter.clone())))
//!         })
//!         .bind("localhost:8080")?
//!         .run()
//!         .await
//! }
//! #[cfg(not(feature = "metrics-prometheus"))]
//! # fn main() {}
//! ```
//!
//! ### Exporter configuration
//!
//! [`actix-web`] uses [`tokio`] as the underlying executor, so exporters should be
//! configured to be non-blocking:
//!
//! ```toml
//! [dependencies]
//! # if exporting to jaeger, use the `tokio` feature.
//! opentelemetry-jaeger = { version = "..", features = ["rt-tokio-current-thread"] }
//!
//! # if exporting to zipkin, use the `tokio` based `reqwest-client` feature.
//! opentelemetry-zipkin = { version = "..", features = ["reqwest-client"], default-features = false }
//!
//! # ... ensure the same same for any other exporters
//! ```
//!
//! [`actix-web`]: https://crates.io/crates/actix-web
//! [`tokio`]: https://crates.io/crates/tokio
#![deny(missing_docs, unreachable_pub, missing_debug_implementations)]
#![cfg_attr(test, deny(warnings))]
#![cfg_attr(docsrs, feature(doc_cfg), deny(broken_intra_doc_links))]

#[cfg(feature = "awc")]
mod client;
mod middleware;
pub(crate) mod util;

#[cfg(feature = "awc")]
#[cfg_attr(docsrs, doc(cfg(feature = "awc")))]
pub use client::{ClientExt, InstrumentedClientRequest};

#[cfg(feature = "metrics-prometheus")]
#[cfg_attr(docsrs, doc(feature = "metrics-prometheus"))]
pub use middleware::metrics::prometheus::PrometheusMetricsHandler;
#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
pub use middleware::metrics::{RequestMetrics, RequestMetricsBuilder, RequestMetricsMiddleware};
pub use {middleware::route_formatter::RouteFormatter, middleware::trace::RequestTracing};
