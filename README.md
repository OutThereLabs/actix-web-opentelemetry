# Actix Web OpenTelemetry

[![Build Status](https://github.com/OutThereLabs/actix-web-opentelemetry/workflows/CI/badge.svg)](https://github.com/OutThereLabs/actix-web-opentelemetry/actions?query=workflow%3ACI)
[![Crates.io: actix-web-opentelemetry](https://img.shields.io/crates/v/actix-web-opentelemetry.svg)](https://crates.io/crates/actix-web-opentelemetry)
[![Documentation](https://docs.rs/actix-web-opentelemetry/badge.svg)](https://docs.rs/actix-web-opentelemetry)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE.txt)

[OpenTelemetry](https://opentelemetry.io/) integration for [Actix Web](https://actix.rs/).

### Exporter configuration

[`actix-web`] uses [`tokio`] as the underlying executor, so exporters should be
configured to be non-blocking:

```toml
[dependencies]
# if exporting to jaeger, use the `tokio` feature.
opentelemetry-jaeger = { version = "..", features = ["rt-tokio-current-thread"] }

# if exporting to zipkin, use the `tokio` based `reqwest-client` feature.
opentelemetry-zipkin = { version = "..", features = ["reqwest-client"], default-features = false }

# ... ensure the same same for any other exporters
```

[`actix-web`]: https://crates.io/crates/actix-web
[`tokio`]: https://crates.io/crates/tokio

### Execute client and server example

```console
# Run jaeger in background
$ docker run -d -p6831:6831/udp -p6832:6832/udp -p16686:16686 jaegertracing/all-in-one:latest

# Run server example with tracing middleware
$ cargo run --example server
# (In other tab) Run client example with request tracing
$ cargo run --example client --features awc

# View spans (see the image below)
$ firefox http://localhost:16686/
```

![Jaeger UI](trace.png)

### Features

- `awc` -- enable support for tracing the `awc` http client.
- `metrics` -- enable support for opentelemetry metrics (only traces are enabled by default)
- `metrics-prometheus` -- enable support for prometheus metrics (requires `metrics` feature)
- `sync-middleware` -- enable tracing on actix-web middlewares that do synchronous work before returning a future. Adds a small amount of overhead to every request.
