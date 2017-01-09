
use super::iron::prelude::*;
use super::router::Router;
use super::iron::status;

pub fn start(addr_and_port: &str) -> Result<(), String> {
    let mut router = Router::new();

    router.post("/", webhook_handler, "webhook_old");
    router.post("/hook", webhook_handler, "webhook");

    match Iron::new(router).http(addr_and_port) {
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
