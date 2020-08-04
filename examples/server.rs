use actix_web::{dev, http, web, App, HttpRequest, HttpServer};
use actix_web_opentelemetry::{RequestMetrics, RequestTracing, UuidWildcardFormatter};
use opentelemetry::{api::KeyValue, global, sdk};
use std::sync::Arc;

async fn index(_req: HttpRequest, _path: actix_web::web::Path<String>) -> &'static str {
    "Hello world!"
}

fn init_tracer() -> std::io::Result<()> {
    let exporter: opentelemetry_jaeger::Exporter = opentelemetry_jaeger::Exporter::builder()
        .with_agent_endpoint("127.0.0.1:6831".parse().unwrap())
        .with_process(opentelemetry_jaeger::Process {
            service_name: "actix_server".to_string(),
            tags: Vec::new(),
        })
        .init()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let provider = sdk::Provider::builder()
        .with_simple_exporter(exporter)
        .with_config(sdk::Config {
            default_sampler: Box::new(sdk::Sampler::AlwaysOn),
            resource: Arc::new(sdk::Resource::new(vec![
                KeyValue::new("service.name", "demo-backend"),
                KeyValue::new("service.namespace", "demo"),
                KeyValue::new("service.instance.id", "1"),
                KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            ])),
            ..Default::default()
        })
        .build();
    global::set_provider(provider);

    Ok(())
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    init_tracer()?;
    let meter = sdk::Meter::new("actix_server");
    let request_metrics = RequestMetrics::new(
        meter,
        UuidWildcardFormatter::new(),
        Some(|req: &dev::ServiceRequest| {
            req.path() == "/metrics" && req.method() == http::Method::GET
        }),
    );
    HttpServer::new(move || {
        App::new()
            .wrap(request_metrics.clone())
            .wrap(RequestTracing::default())
            .service(web::resource("/users/{id}").to(index))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
