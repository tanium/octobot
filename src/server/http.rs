use futures::future::{self, Future};
use futures::Stream;
use hyper::{self, StatusCode};
use hyper::server::{Request, Response};
use serde::de::DeserializeOwned;
use serde_json;

pub type FutureResponse = Box<Future<Item=Response, Error=hyper::Error>>;

pub trait Handler {
    fn handle(&self, req: Request) -> FutureResponse;

    fn respond(&self, resp: Response) -> FutureResponse {
        Box::new(future::ok(resp))
    }

    fn respond_with(&self, status: hyper::StatusCode, msg: &str) -> FutureResponse {
        self.respond(Response::new().with_status(status).with_body(msg.to_string()))
    }
}

pub trait Filter {
    fn filter(&self, req: &Request) -> FilterResult;
}

pub enum FilterResult {
    Halt(Response),
    Continue,
}

pub struct FilteredHandler {
    filter: Box<Filter>,
    handler: Box<Handler>,
}

pub struct NotFoundHandler;

impl FilteredHandler {
    pub fn new(filter: Box<Filter>, handler: Box<Handler>) -> Box<FilteredHandler> {
        Box::new(FilteredHandler {
            filter: filter,
            handler: handler,
        })
    }
}

impl Handler for FilteredHandler {
    fn handle(&self, req: Request) -> FutureResponse {
        match self.filter.filter(&req) {
            FilterResult::Halt(resp) => Box::new(future::ok(resp)),
            FilterResult::Continue => self.handler.handle(req),
        }
    }
}

impl Handler for NotFoundHandler {
    fn handle(&self, _: Request) -> FutureResponse {
        Box::new(future::ok(Response::new().with_status(StatusCode::NotFound)))
    }
}

pub fn parse_json<T: DeserializeOwned, F>(req: Request, func: F) -> FutureResponse
    where F: FnOnce(T) -> Response + 'static
{
    Box::new(req.body().concat2().map(move |data| {
        let obj: T = match serde_json::from_slice(&data) {
            Ok(l) => l,
            Err(e) => return Response::new().with_status(StatusCode::BadRequest)
                                            .with_body(format!("Failed to parse JSON: {}", e)),
        };

        func(obj)
    }))
}
