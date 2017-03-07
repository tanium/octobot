use bodyparser;
use iron::prelude::*;
use iron::status;
use iron::headers::ContentType;
use iron::middleware::Handler;
use iron::modifiers::Header;
use serde_json;

pub struct LoginHandler {
}

impl LoginHandler {
    pub fn new() -> LoginHandler {
        LoginHandler {
        }
    }
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

impl Handler for LoginHandler {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let body = match req.get::<bodyparser::Raw>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                error!("Error reading json");
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))));
            }
        };

        let login_req: LoginRequest = match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(e) => {
                error!("Error parsing login request: {}", e);
                return Ok(Response::with((status::BadRequest,
                                          format!("Error parsing JSON: {}", e))));
            }
        };

        println!("LOGGING IN {} -- {}", login_req.username, login_req.password);

        Ok(Response::with((status::Ok, Header(ContentType::json()), "{}")))
    }
}

