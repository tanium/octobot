use hyper::body::Incoming;
use hyper::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use hyper::{Request, Response, StatusCode};
use rust_embed::RustEmbed;

use crate::http_util::{self, Body};
use crate::server::http::Handler;
use octobot_lib::errors::Result;

// The web UI, built into ui/dist by `npm run build`. Release builds embed it
// and refuse to compile without it. Debug builds read it from disk on each
// request, so cargo works without a UI build.
#[derive(RustEmbed)]
#[folder = "../ui/dist"]
#[cfg_attr(debug_assertions, allow_missing = true)]
struct UiAssets;

pub struct UiHandler;

impl UiHandler {
    pub fn new() -> Box<UiHandler> {
        Box::new(UiHandler)
    }
}

#[async_trait::async_trait]
impl Handler for UiHandler {
    async fn handle(&self, req: Request<Incoming>) -> Result<Response<Body>> {
        let path = match req.uri().path().trim_start_matches('/') {
            "" => "index.html",
            p => p,
        };

        // Only embedded files can be served, so there is no path to traverse.
        let Some(file) = UiAssets::get(path) else {
            return Ok(http_util::new_empty_resp(StatusCode::NOT_FOUND));
        };

        // Vite puts a content hash in every file name under assets/, so those
        // never change. index.html must be revalidated to pick up new hashes.
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };

        let mut resp = Response::new(Body::from(file.data.into_owned()));
        let headers = resp.headers_mut();
        headers.insert(CONTENT_TYPE, HeaderValue::from_str(file.metadata.mimetype())?);
        headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
        Ok(resp)
    }
}
