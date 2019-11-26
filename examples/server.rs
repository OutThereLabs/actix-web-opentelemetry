use actix_web::{dev, http, web, App, HttpRequest, HttpServer};
use actix_web_opentelemetry::{RequestMetrics, RequestTracing, UuidWildcardFormatter};
use opentelemetry::{exporter::trace::jaeger, sdk};

fn index(_req: HttpRequest) -> &'static str {
    "Hello world!"
}

fn init_tracer() {
    let exporter = jaeger::Exporter::builder()
        .with_collector_endpoint("127.0.0.1:6831".parse().unwrap())
        .with_process(jaeger::Process {
            service_name: "actix-server",
            tags: Vec::new(),
        })
        .init();
    let provider = sdk::Provider::builder().with_exporter(exporter).build();
    opentelemetry::global::set_provider(provider);
}

fn main() -> std::io::Result<()> {
    init_tracer();
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
            .service(web::resource("/index.html").to(|| "Hello world!"))
            .service(web::resource("/").to(index))
    })
    .bind("127.0.0.1:8080")?
    .run()
}
