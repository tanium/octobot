use super::*;

use super::iron::prelude::*;
use super::iron::status;
use super::router::Router;
use super::super::logger::Logger;
use super::bodyparser;

use server::github_verify;

pub fn start(config: Config) -> Result<(), String> {
    let mut router = Router::new();
    router.post("/", webhook_handler, "webhook");

    let default_listen = String::from("0.0.0.0:3000");
    let addr_and_port = match config.listen_addr {
        Some(ref addr_and_port) => addr_and_port,
        None => &default_listen,
    };

    let mut chain = Chain::new(router);
    let (logger_before, logger_after) = Logger::new(None);

    // before first middleware
    chain.link_before(logger_before);

    chain.link_before(github_verify::GithubWebhookVerifier { secret: config.github_secret.clone() });

    // after last middleware
    chain.link_after(logger_after);

    match Iron::new(chain).http(addr_and_port.as_str()) {
        Ok(_) => {
            println!("Listening on port {}", addr_and_port);
            Ok(())
        }
        Err(e) => Err(format!("{}", e)),
    }
}


fn webhook_handler(req: &mut Request) -> IronResult<Response> {
    let json_body = match req.get::<bodyparser::Json>() {
        Ok(Some(j)) => j,
        Err(_) | Ok(None) => {
            return Ok(Response::with((status::BadRequest, format!("Error parsing json"))))
        }
    };

    println!("BODY: {:?}", json_body);

    Ok(Response::with((status::Ok, "Hello, Octobot!")))
}
