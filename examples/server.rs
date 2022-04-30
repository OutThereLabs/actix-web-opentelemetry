use actix_web::{web, App, HttpRequest, HttpServer};
use actix_web_opentelemetry::{PrometheusMetricsHandler, RequestMetricsBuilder, RequestTracing};
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

    let meter = opentelemetry::global::meter("actix_web");
    let request_metrics = RequestMetricsBuilder::new().build(meter);

    // Start a new prometheus metrics pipeline if --features metrics-prometheus is used
    #[cfg(feature = "metrics-prometheus")]
    let metrics_handler = {
        let exporter = opentelemetry_prometheus::exporter().init();
        PrometheusMetricsHandler::new(exporter)
    };

    HttpServer::new(move || {
        let app = App::new()
            .wrap(RequestTracing::new())
            .wrap(request_metrics.clone())
            .service(web::resource("/users/{id}").to(index));

        #[cfg(feature = "metrics-prometheus")]
        let app = app.route("/metrics", web::get().to(metrics_handler.clone()));

        app
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await?;

    // Ensure all spans have been reported
    opentelemetry::global::shutdown_tracer_provider();

    Ok(())
}
