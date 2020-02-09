use actix_rt::System;
use actix_web::client;
use opentelemetry::{global, sdk};
use std::io;
use std::thread;
use std::time::Duration;

async fn execute_request(client: client::Client) -> Result<String, io::Error> {
    let mut response =
        actix_web_opentelemetry::with_tracing(client.get("http://localhost:8080"), |request| {
            request.send()
        })
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

fn init_tracer() -> io::Result<()> {
    let exporter: opentelemetry_jaeger::Exporter = opentelemetry_jaeger::Exporter::builder()
        .with_agent_endpoint("127.0.0.1:6831".parse().unwrap())
        .with_process(opentelemetry_jaeger::Process {
            service_name: "actix_client".to_string(),
            tags: Vec::new(),
        })
        .init()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let provider = sdk::Provider::builder()
        .with_simple_exporter(exporter)
        .with_config(sdk::Config {
            default_sampler: Box::new(sdk::Sampler::Always),
            ..Default::default()
        })
        .build();
    global::set_provider(provider);

    Ok(())
}

fn main() -> io::Result<()> {
    init_tracer()?;
    let client = client::Client::new();
    let response = System::new("actix-web-opentelemetry").block_on(execute_request(client))?;

    println!("Response: {}", response);

    thread::sleep(Duration::from_millis(100));

    Ok(())
}
