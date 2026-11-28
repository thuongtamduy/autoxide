//! The HTTP layer's own measurements. Serving them at `/metrics` is the binary's job (`autoxide-host`).

/// Records measurements for a finished request. `route` is the route PATTERN (e.g.
/// `/api/v1/notes/{id}`), not the raw path - so the number of metric series does not grow
/// with the number of IDs.
pub(super) fn record_request(method: &str, route: &str, status: u16, elapsed_seconds: f64) {
    // HTTP nhận method bất kỳ (`FOO`, `M123`…): nhãn chỉ lấy trong tập cố định, không thì mỗi
    // method lạ thêm một chuỗi số đo sống mãi trong bộ nhớ (D-157).
    let method = match method {
        "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" => method,
        _ => "other",
    };
    let labels = [
        ("method", method.to_owned()),
        ("route", route.to_owned()),
        ("status", status.to_string()),
    ];
    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_request_duration_seconds", &labels[..2]).record(elapsed_seconds);
}
