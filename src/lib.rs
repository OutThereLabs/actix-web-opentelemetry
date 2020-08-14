//! # Actix Web OpenTelemetry
//!
//! [OpenTelemetry](https://opentelemetry.io/) integration for [Actix Web](https://actix.rs/).
//!
//! This crate allows you to easily instrument client and server requests.
//!
//! * Client requests can be traced by using the [`ClientExt::trace_request`] function.
//! * Server requests can be traced by using the [`RequestTracing`] struct.
//!
//! [`ClientExt::trace_request`]: trait.ClientExt.html#method.trace_request
//! [`RequestTracing`]: struct.RequestTracing.html
//!
//! ### Client Request Example:
//! ```no_run
//! use actix_web::client;
//! use actix_web_opentelemetry::ClientExt;
//! use futures::Future;
//!
//! async fn execute_request(client: &client::Client) -> Result<(), client::SendRequestError> {
//!     let res = client
//!         .get("http://localhost:8080")
//!         .trace_request()
//!         .send()
//!         .await?;
//!
//!     println!("Response: {:?}", res);
//!     Ok(())
//! }
//! ```
//!
//! ### Server middleware example:
//! ```no_run
//! use actix_web::{web, App, HttpServer};
//! use actix_web_opentelemetry::RequestTracing;
//! use opentelemetry::api;
//!
//! fn init_tracer() {
//!     // Replace this no-op provider with something like:
//!     // https://docs.rs/opentelemetry-jaeger
//!     opentelemetry::global::set_provider(api::NoopProvider {});
//! }
//!
//! async fn index() -> &'static str {
//!     "Hello world!"
//! }
//!
//! #[actix_rt::main]
//! async fn main() -> std::io::Result<()> {
//!     init_tracer();
//!     HttpServer::new(|| {
//!         App::new()
//!             .wrap(RequestTracing::default())
//!             .service(web::resource("/").to(index))
//!     })
//!     .bind("127.0.0.1:8080")?
//!     .run()
//!     .await
//! }
//! ```
//!
#![deny(missing_docs, unreachable_pub, missing_debug_implementations)]
#![cfg_attr(test, deny(warnings))]

mod client;
mod middleware;

#[allow(deprecated)]
pub use {
    client::{with_tracing, ClientExt, InstrumentedClientRequest},
    middleware::metrics::{RequestMetrics, RequestMetricsMiddleware},
    middleware::route_formatter::{RouteFormatter, UuidWildcardFormatter},
    middleware::trace::RequestTracing,
};
