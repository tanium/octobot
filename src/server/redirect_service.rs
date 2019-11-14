use futures::future::{self, FutureResult};
use http::header::{HeaderMap, HeaderValue};
use hyper::{self, Body, Request, Response};
use hyper::{StatusCode, Uri};
use hyper::header::{HOST, LOCATION};
use hyper::service::{NewService, Service};
use log::{debug, error};

use crate::util;

#[derive(Clone)]
pub struct RedirectService {
    https_port: u16,
}

impl RedirectService {
    pub fn new(https_port: u16) -> RedirectService {
        RedirectService { https_port: https_port }
    }

    fn rewrite_uri(&self, uri: Uri, host_header: Option<Uri>) -> String {
        let mut new_url = String::from("https://");
        if let Some(host) = uri.host() {
            new_url += host;
            self.maybe_add_port(&mut new_url, uri.port_part())
        } else if let Some(host_header) = host_header {
            if let Some(host) = host_header.host() {
                new_url += host;
                self.maybe_add_port(&mut new_url, host_header.port_part());
            }
        }
        new_url += uri.path();
        if let Some(q) = uri.query() {
            new_url += &format!("?{}", q);
        }

        new_url
    }

    fn maybe_add_port(&self, new_url: &mut String, req_port: Option<http::uri::Port<&str>>) {
        // if port was specified, then not using docker or otherwise to remap ports --> substitute explicit port
        if req_port.is_some() {
            new_url.push_str(&format!(":{}", self.https_port));
        }
    }
}

impl NewService for RedirectService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Service = RedirectService;
    type Future = future::FutureResult<RedirectService, hyper::Error>;
    type InitError = hyper::Error;

    fn new_service(&self) -> Self::Future {
        future::ok(self.clone())
    }
}

impl Service for RedirectService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Future = FutureResult<Response<Body>, hyper::Error>;

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let host_header = get_host_header(&req.headers());

        let new_uri_str = self.rewrite_uri(req.uri().clone(), host_header);
        let new_uri = match HeaderValue::from_str(&new_uri_str) {
            Err(e) => {
                error!("Invalid Location header '{}': {}", new_uri_str, e);
                return future::ok(util::new_empty_resp(StatusCode::INTERNAL_SERVER_ERROR));
            }
            Ok(uri) => uri,
        };

        debug!("Redirecting request to {}", new_uri_str);
        let mut resp = util::new_empty_resp(StatusCode::MOVED_PERMANENTLY);
        resp.headers_mut().insert(LOCATION, new_uri);

        future::ok(resp)
    }
}

fn get_host_header(headers: &HeaderMap) -> Option<Uri> {
    headers.get(HOST).and_then(|h| h.to_str().ok()).and_then(|h| h.parse::<Uri>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_rewrite_uri_uri_host_primary() {
        let service = RedirectService::new(99);
        let uri = Uri::from_str("http://host.foo.com/path/to/thing?param=value&param2=value2").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "other.com".parse().unwrap());

        assert_eq!(
            "https://host.foo.com/path/to/thing?param=value&param2=value2",
            service.rewrite_uri(uri, get_host_header(&headers))
        );
    }

    #[test]
    fn test_rewrite_uri_header_host_secondary() {
        let service = RedirectService::new(99);
        let uri = Uri::from_str("/path/to/thing?param=value&param2=value2").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "other.com".parse().unwrap());

        assert_eq!(
            "https://other.com/path/to/thing?param=value&param2=value2",
            service.rewrite_uri(uri, get_host_header(&headers))
        );
    }

    #[test]
    fn test_rewrite_uri_includes_port_if_uri_has_port() {
        let service = RedirectService::new(99);
        let uri = Uri::from_str("http://host.foo.com:20/path/to/thing?param=value&param2=value2").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "other.com".parse().unwrap());

        assert_eq!(
            "https://host.foo.com:99/path/to/thing?param=value&param2=value2",
            service.rewrite_uri(uri, get_host_header(&headers))
        );
    }

    #[test]
    fn test_rewrite_uri_includes_port_if_header_has_port() {
        let service = RedirectService::new(99);
        let uri = Uri::from_str("/path/to/thing?param=value&param2=value2").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "other.com:20".parse().unwrap());

        assert_eq!(
            "https://other.com:99/path/to/thing?param=value&param2=value2",
            service.rewrite_uri(uri, get_host_header(&headers))
        );
    }
}
