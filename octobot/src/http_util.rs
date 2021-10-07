use std::env;
use hyper::{self, Body, Response, StatusCode};

pub fn is_dev_mode() -> bool {
    env::var("DEVMODE").is_ok()
}

pub fn new_msg_resp<S: Into<String>>(status: StatusCode, msg: S) -> Response<Body> {
    let msg: String = msg.into();
    let mut resp = Response::new(Body::from(msg));
    *resp.status_mut() = status;
    resp
}

pub fn new_json_resp(json: String) -> Response<Body> {
    let mut resp = Response::new(Body::from(json));
    resp.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );
    resp
}

pub fn new_empty_resp(status: StatusCode) -> Response<Body> {
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = status;
    resp
}

pub fn new_empty_error_resp() -> Response<Body> {
    new_empty_resp(StatusCode::INTERNAL_SERVER_ERROR)
}

pub fn new_error_resp<S: Into<String>>(msg: S) -> Response<Body> {
    new_msg_resp(StatusCode::INTERNAL_SERVER_ERROR, msg)
}

pub fn new_bad_req_resp<S: Into<String>>(msg: S) -> Response<Body> {
    new_msg_resp(StatusCode::BAD_REQUEST, msg)
}
