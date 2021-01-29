use actix_web::client;
use actix_web_opentelemetry::ClientExt;
use opentelemetry::{
    global,
    sdk::{
        export::trace::SpanExporter,
        propagation::TraceContextPropagator,
        trace::{BatchSpanProcessor, TracerProvider},
    },
};
use std::error::Error;
use std::io;

async fn execute_request(client: client::Client) -> io::Result<String> {
    let mut response = client
        .get("http://localhost:8080/users/103240ba-3d8d-4695-a176-e19cbc627483?a=1")
        .trace_request()
        .send()
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

    let bytes = response
        .body()
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

    std::str::from_utf8(&bytes)
        .map(|s| s.to_owned())
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
}

// Compatibility with older tokio v0.2.x used by actix web v3. Not necessary with actic web v4.
fn tokio_exporter_compat<T: SpanExporter + 'static>(exporter: T) -> BatchSpanProcessor {
    let spawn = |fut| tokio::task::spawn_blocking(|| futures::executor::block_on(fut));
    BatchSpanProcessor::builder(
        exporter,
        spawn,
        tokio::time::sleep,
        tokio::time::interval,
    )
    .build()
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    global::set_text_map_propagator(TraceContextPropagator::new());
    let exporter = opentelemetry_jaeger::new_pipeline()
        .with_service_name("actix_client")
        .init_exporter()
        .expect("pipeline exporter error");
    let tracer_provider = TracerProvider::builder()
        .with_batch_exporter(tokio_exporter_compat(exporter))
        .build();
    let _uninstall = global::set_tracer_provider(tracer_provider);

    let client = client::Client::new();
    let response = execute_request(client).await?;

    println!("Response: {}", response);

    Ok(())
}
