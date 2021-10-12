use async_trait::async_trait;
use hyper::{self, Body, Request, Response, StatusCode};
use log::error;
use serde::de::DeserializeOwned;
use serde_json;

use octobot_lib::errors::*;
use crate::http_util;

#[async_trait]
pub trait Handler : Send + Sync {
    async fn handle_ok(&self, req: Request<Body>) -> Response<Body> {
        match self.handle(req).await {
            Ok(r) => r,
            Err(e) => {
                error!("Request handler error: {}", e);
                http_util::new_empty_error_resp()
            }
        }
    }

    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>>;

    fn respond_with(&self, status: StatusCode, msg: &str) -> Response<Body> {
        http_util::new_msg_resp(status, msg.to_string())
    }

    fn respond_error(&self, err: &str) -> Response<Body> {
        error!("InternalServerError: {}", err);
        http_util::new_empty_resp(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

pub trait Filter : Send + Sync {
    fn filter(&self, req: Request<Body>) -> FilterResult;
}

pub enum FilterResult {
    Halt(Response<Body>),
    Continue(Request<Body>),
}

pub struct FilteredHandler {
    filter: Box<dyn Filter>,
    handler: Box<dyn Handler>,
}

pub struct NotFoundHandler;

impl FilteredHandler {
    pub fn new(filter: Box<dyn Filter>, handler: Box<dyn Handler>) -> Box<FilteredHandler> {
        Box::new(FilteredHandler {
            filter,
            handler,
        })
    }
}

#[async_trait]
impl Handler for FilteredHandler {
    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>> {
        let req = match self.filter.filter(req) {
            FilterResult::Halt(resp) => return Ok(resp),
            FilterResult::Continue(req) => req,
        };

        self.handler.handle(req).await
    }
}

#[async_trait]
impl Handler for NotFoundHandler {
    async fn handle(&self, _: Request<Body>) -> Result<Response<Body>> {
        Ok(http_util::new_empty_resp(StatusCode::NOT_FOUND))
    }
}

pub async fn parse_json<T: DeserializeOwned>(req: Request<Body>) -> Result<T> {
    let bytes = match hyper::body::to_bytes(req.into_body()).await {
        Ok(b) => b,
        Err(e) => {
            return Err(failure::format_err!("Failed to read request body: {}", e));
        }
    };

    serde_json::from_slice::<T>(bytes.as_ref())
        .map_err(|e| failure::format_err!("Failed to parse JSON: {}", e))
}
