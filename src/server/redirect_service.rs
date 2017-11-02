use futures::future::{self, FutureResult};
use hyper;
use hyper::{StatusCode, Uri};
use hyper::header::{Host, Location};
use hyper::server::{Request, Response, Service};

pub struct RedirectService {
    https_port: u16,
}

impl RedirectService {
    pub fn new(https_port: u16) -> RedirectService {
        RedirectService { https_port: https_port }
    }

    fn rewrite_uri(&self, uri: Uri, host_header: Option<&Host>) -> String {
        let mut new_url = String::from("https://");
        if let Some(host) = uri.host() {
            new_url += host;
            self.maybe_add_port(&mut new_url, uri.port())
        } else if let Some(host) = host_header {
            new_url += host.hostname();
            self.maybe_add_port(&mut new_url, host.port());
        }
        new_url += uri.path();
        if let Some(q) = uri.query() {
            new_url += &format!("?{}", q);
        }

        new_url
    }

    fn maybe_add_port(&self, new_url: &mut String, req_port: Option<u16>) {
        // if port was specified, then not using docker or otherwise to remap ports --> substitute explicit port
        if req_port.is_some() {
            new_url.push_str(&format!(":{}", self.https_port));
        }
    }
}

impl Service for RedirectService {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = FutureResult<Response, hyper::Error>;

    fn call(&self, req: Request) -> Self::Future {
        let new_uri = self.rewrite_uri(req.uri().clone(), req.headers().get::<Host>());
        debug!("Redirecting request to {}", new_uri);
        future::ok(Response::new().with_status(StatusCode::MovedPermanently).with_header(Location::new(new_uri)))
    }
}
