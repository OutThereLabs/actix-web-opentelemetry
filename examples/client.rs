use actix_web_opentelemetry::ClientExt;
use opentelemetry::{global, KeyValue};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use std::error::Error;
use std::io;
use opentelemetry_otlp::TonicExporterBuilder;
use opentelemetry_sdk::{Resource, trace};
use opentelemetry_sdk::runtime::TokioCurrentThread;

async fn execute_request(client: awc::Client) -> io::Result<String> {
    let mut response = client
        .get("http://127.0.0.1:8080/users/103240ba-3d8d-4695-a176-e19cbc627483?a=1")
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
    // Start a new OTLP trace pipeline
    global::set_text_map_propagator(TraceContextPropagator::new());

    let service_name_resource = Resource::new(vec![KeyValue::new(
        "service.name",
        "actix_client"
    )]);

    let _tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(TonicExporterBuilder::default())
        .with_trace_config(
            trace::Config::default()
                .with_resource(service_name_resource)
        )
        .install_batch(TokioCurrentThread)
        .expect("pipeline install error");

    let client = awc::Client::new();
    let response = execute_request(client).await?;

    println!("Response: {}", response);

    global::shutdown_tracer_provider();

    Ok(())
}
