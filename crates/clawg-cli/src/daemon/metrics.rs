//! What the daemon measures of itself: every HTTP request here, and (in
//! `serve`) the writer that keeps each minute in `metrics.db`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::Response;
use clawg_core::metrics::{Recorder, Unit};

/// Which listener a request came in on. The router is the same for both, so
/// `serve` marks each server's copy with this as an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Unix,
    Tcp,
}

impl Transport {
    fn as_str(self) -> &'static str {
        match self {
            Transport::Unix => "unix",
            Transport::Tcp => "tcp",
        }
    }
}

/// The recorder and the requests in flight, shared by every copy of a
/// router.
pub struct HttpMeter {
    recorder: Arc<Recorder>,
    in_flight: AtomicU64,
}

impl HttpMeter {
    pub fn new(recorder: Arc<Recorder>) -> Self {
        Self { recorder, in_flight: AtomicU64::new(0) }
    }
}

/// Takes one off the requests in flight when the request ends, also when
/// its future is dropped because the client went away.
struct InFlight<'a>(&'a HttpMeter);

impl InFlight<'_> {
    fn enter(meter: &HttpMeter) -> InFlight<'_> {
        let now = meter.in_flight.fetch_add(1, Ordering::Relaxed) + 1;
        meter.recorder.gauge("http.in_flight", &[], now as f64);
        InFlight(meter)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let now = self.0.in_flight.fetch_sub(1, Ordering::Relaxed).saturating_sub(1);
        self.0.recorder.gauge("http.in_flight", &[], now as f64);
    }
}

/// Times every request, those the guard or the token refuse included. Every
/// label comes from a closed set or from the route template axum matched,
/// never from the URL, the query or the body.
pub async fn time_request(State(meter): State<Arc<HttpMeter>>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = method_label(req.method());
    let route = req.extensions().get::<MatchedPath>().map(|p| p.as_str().to_string());
    let route = route.as_deref().unwrap_or("unmatched").to_string();
    let transport = req.extensions().get::<Transport>().copied().unwrap_or(Transport::Tcp);
    meter.recorder.observe(
        "http.request_bytes",
        Unit::Bytes,
        &[("method", method), ("route", &route)],
        content_length(req.headers()),
    );

    let in_flight = InFlight::enter(&meter);
    let response = next.run(req).await;
    drop(in_flight);

    let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    meter.recorder.observe(
        "http.duration",
        Unit::Micros,
        &[
            ("method", method),
            ("route", &route),
            ("status", status_class(response.status().as_u16())),
            ("transport", transport.as_str()),
        ],
        micros,
    );
    response
}

fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::PUT => "PUT",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        _ => "other",
    }
}

fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

fn content_length(headers: &HeaderMap) -> u64 {
    headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_falls_in_its_class() {
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(401), "4xx");
        assert_eq!(status_class(503), "5xx");
        assert_eq!(status_class(101), "1xx");
    }

    #[test]
    fn a_size_without_a_valid_content_length_is_zero() {
        let mut headers = HeaderMap::new();
        assert_eq!(content_length(&headers), 0);
        headers.insert(axum::http::header::CONTENT_LENGTH, "12".parse().unwrap());
        assert_eq!(content_length(&headers), 12);
        headers.insert(axum::http::header::CONTENT_LENGTH, "lots".parse().unwrap());
        assert_eq!(content_length(&headers), 0);
    }
}
