use std::env;
use std::io::Read;
use std::fs::File;

use iron::prelude::*;
use iron::status;
use iron::headers::ContentType;
use iron::middleware::Handler;
use iron::modifiers::Header;

fn is_dev_mode() -> bool {
    env::var("DEVMODE").is_ok()
}

pub struct HtmlHandler {
    path: String,
    contents: String
}

impl HtmlHandler {
    pub fn new(path: &str, contents: &str)  -> HtmlHandler {
        HtmlHandler {
            path: path.into(),
            contents: contents.into()
        }
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
    fn handle(&self, _: &mut Request) -> IronResult<Response> {
        Ok(Response::with((status::Ok, Header(ContentType::html()), self.contents())))
    }
}

