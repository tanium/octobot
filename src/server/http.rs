use super::*;

use super::iron::prelude::*;
use super::iron::status;
use super::router::Router;
use super::super::logger::Logger;

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
        },
        Err(e) => Err(format!("{}", e)),
    }
}


fn webhook_handler(_: &mut Request) -> IronResult<Response> {
    Ok(Response::with((status::Ok, "Hello, Octobot!")))
}
