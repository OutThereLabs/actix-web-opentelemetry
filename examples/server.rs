use actix_web::{web, App, HttpRequest, HttpServer};
use actix_web_opentelemetry::RequestTracing;
use opentelemetry::{
    global,
    sdk::{
        export::trace::SpanExporter,
        propagation::TraceContextPropagator,
        trace::{BatchSpanProcessor, TracerProvider},
    },
};
use std::io;

async fn index(_req: HttpRequest, _path: actix_web::web::Path<String>) -> &'static str {
    "Hello world!"
}

// Compatibility with older tokio v0.2.x used by actix web v3. Not necessary with actix web v4.
fn tokio_exporter_compat<T: SpanExporter + 'static>(exporter: T) -> BatchSpanProcessor {
    let spawn = |fut| tokio::task::spawn_blocking(|| futures::executor::block_on(fut));
    BatchSpanProcessor::builder(
        exporter,
        spawn,
        tokio::time::delay_for,
        tokio::time::interval,
    )
    .build()
}

#[actix_web::main]
async fn main() -> io::Result<()> {
    // Start a new jaeger trace pipeline
    global::set_text_map_propagator(TraceContextPropagator::new());
    let exporter = opentelemetry_jaeger::new_pipeline()
        .with_service_name("actix_server")
        .init_exporter()
        .expect("pipeline install error");
    let tracer_provider = TracerProvider::builder()
        .with_batch_exporter(tokio_exporter_compat(exporter))
        .build();
    let _uninstall = global::set_tracer_provider(tracer_provider);

    // Start a new prometheus metrics pipeline if --features metrics is used
    #[cfg(feature = "metrics")]
    let exporter = opentelemetry_prometheus::exporter().init();

    #[cfg(feature = "metrics")]
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

        #[cfg(feature = "metrics")]
        let app = app.wrap(request_metrics.clone());

        app
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
