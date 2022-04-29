use actix_web::{web, App, HttpRequest, HttpServer};
use actix_web_opentelemetry::{RequestMetricsBuilder, RequestTracing};
use opentelemetry::{
    global, runtime::TokioCurrentThread, sdk::propagation::TraceContextPropagator,
};
use std::io;

async fn index(_req: HttpRequest, _path: actix_web::web::Path<String>) -> &'static str {
    "Hello world!"
}

#[actix_web::main]
async fn main() -> io::Result<()> {
    // Start a new jaeger trace pipeline
    global::set_text_map_propagator(TraceContextPropagator::new());
    let _tracer = opentelemetry_jaeger::new_pipeline()
        .with_service_name("actix_server")
        .install_batch(TokioCurrentThread)
        .expect("pipeline install error");

    let request_metrics = RequestMetricsBuilder::new();

    // Start a new prometheus metrics pipeline if --features metrics is used
    #[cfg(feature = "metrics-prometheus")]
    let exporter = opentelemetry_prometheus::exporter().init();

    let request_metrics = request_metrics.build(opentelemetry::global::meter("actix_web"));

    HttpServer::new(move || {
        let app = App::new()
            .wrap(RequestTracing::new())
            .service(web::resource("/users/{id}").to(index));

        let app = app.wrap(request_metrics.clone());

        #[cfg(feature = "metrics-prometheus")]
        let app = app.route(
            "/metrics",
            web::get().to(request_metrics.route(exporter.clone())),
        );

        app
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await?;

    // Ensure all spans have been reported
    opentelemetry::global::shutdown_tracer_provider();

    Ok(())
}
