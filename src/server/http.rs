use futures::Stream;
use futures::future::{self, Future};
use hyper::{self, Body, Request, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json;

use util;

pub type FutureResponse = Box<dyn Future<Item = Response<Body>, Error = hyper::Error> + Send>;

pub trait Handler {
    fn handle(&self, req: Request<Body>) -> FutureResponse;

    fn respond(&self, resp: Response<Body>) -> FutureResponse {
        Box::new(future::ok(resp))
    }

    fn respond_with(&self, status: StatusCode, msg: &str) -> FutureResponse {
        self.respond(util::new_msg_resp(status, msg.to_string()))
    }

    fn respond_error(&self, err: &str) -> FutureResponse {
        error!("InternalServerError: {}", err);
        self.respond(util::new_empty_resp(StatusCode::INTERNAL_SERVER_ERROR))
    }
}

pub trait Filter {
    fn filter(&self, req: &Request<Body>) -> FilterResult;
}

pub enum FilterResult {
    Halt(Response<Body>),
    Continue,
}

pub struct FilteredHandler {
    filter: Box<dyn Filter>,
    handler: Box<dyn Handler>,
}

pub struct NotFoundHandler;

impl FilteredHandler {
    pub fn new(filter: Box<dyn Filter>, handler: Box<dyn Handler>) -> Box<FilteredHandler> {
        Box::new(FilteredHandler {
            filter: filter,
            handler: handler,
        })
    }
}

impl Handler for FilteredHandler {
    fn handle(&self, req: Request<Body>) -> FutureResponse {
        match self.filter.filter(&req) {
            FilterResult::Halt(resp) => Box::new(future::ok(resp)),
            FilterResult::Continue => self.handler.handle(req),
        }
    }
}

impl Handler for NotFoundHandler {
    fn handle(&self, _: Request<Body>) -> FutureResponse {
        Box::new(future::ok(util::new_empty_resp(StatusCode::NOT_FOUND)))
    }
}

pub fn parse_json<T: DeserializeOwned, F>(req: Request<Body>, func: F) -> FutureResponse
where
    F: FnOnce(T) -> Response<Body> + Send + 'static,
{
    Box::new(req.into_body().concat2().map(move |data| {
        let obj: T = match serde_json::from_slice(&data) {
            Ok(l) => l,
            Err(e) => {
                return util::new_bad_req_resp(format!("Failed to parse JSON: {}", e));
            }
        };

        func(obj)
    }))
}
