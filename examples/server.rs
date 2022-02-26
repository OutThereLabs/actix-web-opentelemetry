use actix_web::{web, App, HttpRequest, HttpServer};
use actix_web_opentelemetry::RequestTracing;
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

    // Start a new prometheus metrics pipeline if --features metrics is used
    let exporter = opentelemetry_prometheus::exporter().init();

    let request_metrics = actix_web_opentelemetry::RequestMetrics::new(
        opentelemetry::global::meter("actix_web"),
        Some(|req: &actix_web::dev::ServiceRequest| {
            req.path() == "/metrics" && req.method() == actix_web::http::Method::GET
        }),
        Some(exporter),
    );

    HttpServer::new(move || {
        let app = App::new()
            .wrap(RequestTracing::new())
            .service(web::resource("/users/{id}").to(index));

        let app = app.wrap(request_metrics.clone());

        app
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await?;

    // Ensure all spans have been reported
    opentelemetry::global::shutdown_tracer_provider();

    Ok(())
}
