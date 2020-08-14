# Changelog

## [v0.5.0](https://github.com/OutThereLabs/actix-web-opentelemetry/compare/v0.4.0...v0.5.0)

### Added

- Trace actix client requests with `ClientExt::trace_request` or
  `ClientExt::trace_request_with_context`. #17

### Changed

- Update to OpenTelemetry v0.8.0 #18
- Deprecated `with_tracing` fn. Use `ClientExt` instead. #17


## [v0.4.0](https://github.com/OutThereLabs/actix-web-opentelemetry/compare/v0.3.0...v0.4.0)

### Changed

- Update to OpenTelemetry v0.7.0 #11

## [v0.3.0](https://github.com/OutThereLabs/actix-web-opentelemetry/compare/v0.2.0...v0.3.0)

### Changed

- Update to OpenTelemetry v0.6.0 #10

## [v0.2.0](https://github.com/OutThereLabs/actix-web-opentelemetry/compare/v0.1.2...v0.2.0)

### Changed

- Update to OpenTelemetry v0.2.0 #6

## [v0.1.2](https://github.com/OutThereLabs/actix-web-opentelemetry/compare/v0.1.1...v0.1.2)

### Changed

- Make client span name match otel spec #3

## [v0.1.1](https://github.com/OutThereLabs/actix-web-opentelemetry/compare/v0.1.0...v0.1.1)

### Added

- Add option for route formatter #1
- Add metrics middleware #2

## [v0.1.0](https://github.com/OutThereLabs/actix-web-opentelemetry/tree/v0.1.0)

Initial debug alpha
