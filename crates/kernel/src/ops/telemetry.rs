//! Reads the caller's trace context (W3C Trace Context, header `traceparent`) for the HTTP layer, so a request's
//! span joins the caller's trace.
//!
//! Sending spans to an OpenTelemetry collector is the binary's job (`autoxide-host`, D-20261128-host-crate): it
//! installs the propagator this file reads through. Until then the result is always an empty context.

use axum::http::HeaderMap;
use opentelemetry::Context;
use opentelemetry::propagation::Extractor;

/// The caller's trace context, read from the request's header (`traceparent`). When trace export is off or the
/// request does not carry that header, the result is an empty context.
pub fn remote_context(headers: &HeaderMap) -> Context {
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderCarrier(headers))
    })
}

/// Lets the OpenTelemetry library read HTTP headers.
struct HeaderCarrier<'a>(&'a HeaderMap);

impl Extractor for HeaderCarrier<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|name| name.as_str()).collect()
    }
}
