# Actix Web OpenTelemetry

[![Crates.io: actix-web-opentelemetry](https://img.shields.io/crates/v/actix-web-opentelemetry.svg)](https://crates.io/crates/actix-web-opentelemetry)
[![Documentation](https://docs.rs/actix-web-opentelemetry/badge.svg)](https://docs.rs/actix-web-opentelemetry)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE.txt)

[OpenTelemetry](https://opentelemetry.io/) integration for [Actix Web](https://actix.rs/).

### Execute client and server example

```console
# Run jaeger in background
$ docker run -d -p6831:6831/udp -p6832:6832/udp -p16686:16686 jaegertracing/all-in-one:latest

# Run server example with tracing middleware
$ cargo run --example server
# (In other tab) Run client example with request tracing
$ cargo run --example client

# View spans (see the image below)
$ firefox http://localhost:16686/
```

![Jaeger UI](trace.png)