use actix_rt::System;
use actix_web::client;
use futures::{lazy, Future};
use opentelemetry::exporter::trace::jaeger;
use opentelemetry::sdk;
use std::thread;
use std::time::Duration;

fn execute_request(client: &client::Client) -> impl Future<Item = String, Error = ()> {
    actix_web_opentelemetry::with_tracing(client.get("http://localhost:8080"), |request| {
        request.send()
    })
    .map_err(|err| eprintln!("Error: {:?}", err))
    .and_then(|mut res| {
        res.body()
            .map(|bytes| std::str::from_utf8(&bytes).unwrap().to_string())
            .map_err(|err| eprintln!("Error: {:?}", err))
    })
}

fn init_tracer() {
    let exporter = jaeger::Exporter::builder()
        .with_collector_endpoint("127.0.0.1:6831".parse().unwrap())
        .with_process(jaeger::Process {
            service_name: "actix-client",
            tags: Vec::new(),
        })
        .init();
    let provider = sdk::Provider::builder().with_exporter(exporter).build();
    opentelemetry::global::set_provider(provider);
}

fn main() {
    init_tracer();
    let client = client::Client::new();
    let _ = System::new("actix-web-opentelemetry").block_on(lazy(|| {
        execute_request(&client).and_then(|response| {
            println!("Response: {}", response);
            Ok(())
        })
    }));

    thread::sleep(Duration::from_millis(100));
}
