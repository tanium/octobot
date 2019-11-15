use std::env;
use std::fs::File;
use std::io::Read;

use hyper::{Body, Request, Response};
use hyper::header::CONTENT_TYPE;

use crate::server::http::{FutureResponse, Handler};

fn is_dev_mode() -> bool {
    env::var("DEVMODE").is_ok()
}

pub struct HtmlHandler {
    path: String,
    contents: String,
}

impl HtmlHandler {
    pub fn new(path: &str, contents: &str) -> Box<HtmlHandler> {
        Box::new(HtmlHandler {
            path: path.into(),
            contents: contents.into(),
        })
    }

    pub fn contents(&self) -> String {
        if is_dev_mode() && self.path.len() > 0 {
            let mut file_contents = String::new();
            let mut file = match File::open(format!("src/assets/{}", self.path)) {
                Ok(f) => f,
                Err(e) => return format!("Error opening file: {}", e),
            };
            if let Err(e) = file.read_to_string(&mut file_contents) {
                return format!("Error reading file: {}", e);
            }

            file_contents
        } else {
            self.contents.clone()
        }
    }
}

impl Handler for HtmlHandler {
    fn handle(&self, _: Request<Body>) -> FutureResponse {
        let mut resp = Response::new(Body::from(self.contents()));
        resp.headers_mut().insert(CONTENT_TYPE, "text/html".parse().unwrap());

        self.respond(resp)
    }
}
