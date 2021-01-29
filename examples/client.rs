use actix_web::client;
use actix_web_opentelemetry::ClientExt;
use opentelemetry::{
    global,
    sdk::{
        propagation::TraceContextPropagator,
        trace::{BatchSpanProcessor, TracerProvider},
    },
    util,
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

#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    global::set_text_map_propagator(TraceContextPropagator::new());
    let exporter = opentelemetry_jaeger::new_pipeline()
        .with_service_name("actix_client")
        .init_exporter()
        .expect("pipeline exporter error");
    let batch_exporter = BatchSpanProcessor::builder(
        exporter,
        tokio::spawn,
        tokio::time::sleep,
        util::tokio_interval_stream,
    )
    .build();
    let tracer_provider = TracerProvider::builder()
        .with_batch_exporter(batch_exporter)
        .build();
    let _uninstall = global::set_tracer_provider(tracer_provider);

    let client = client::Client::new();
    let response = execute_request(client).await?;

    println!("Response: {}", response);

    Ok(())
}
