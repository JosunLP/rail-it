//! Bedienplatz (Spurplan-UI) als statische Seite unter `/ui/`.
//!
//! Die Seite ist ein reiner Client der API: Sie liest `/v1/layout` und `/v1/state` und bedient
//! über dieselben Pfade wie jeder andere Client, mit derselben Anmeldung. Gebaut wird sie mit
//! Bun aus `apps/railitd/ui` (TypeScript, bQuery); `build.rs` bettet das Ergebnis ein (ohne
//! gebauten Bedienplatz eine Hinweisseite).
//!
//! Die Seite selbst ist ohne Anmeldung erreichbar, enthält aber keine Daten. Jede Antwort trägt
//! eine strenge Content-Security-Policy: nur eigene Skripte und Stile, keine Inline-Skripte,
//! keine fremden Verbindungen, keine Einbettung in fremde Seiten.

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;

const INDEX: &str = include_str!(concat!(env!("OUT_DIR"), "/ui/index.html"));
const APP: &str = include_str!(concat!(env!("OUT_DIR"), "/ui/app.js"));
const STYLE: &str = include_str!(concat!(env!("OUT_DIR"), "/ui/style.css"));

/// Content-Security-Policy des Bedienplatzes.
pub(crate) const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     connect-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; \
     base-uri 'none'; form-action 'none'";

fn file(content_type: &'static str, body: &'static str) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Pfade des Bedienplatzes (ohne Anmeldung).
pub(crate) fn router() -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to("/ui/") }))
        .route("/ui", get(|| async { Redirect::to("/ui/") }))
        .route(
            "/ui/",
            get(|| async { file("text/html; charset=utf-8", INDEX) }),
        )
        .route(
            "/ui/app.js",
            get(|| async { file("text/javascript; charset=utf-8", APP) }),
        )
        .route(
            "/ui/style.css",
            get(|| async { file("text/css; charset=utf-8", STYLE) }),
        )
}
