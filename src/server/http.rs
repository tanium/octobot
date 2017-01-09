use super::*;
use super::iron::prelude::*;
use super::router::Router;
use super::iron::status;

pub fn start(config: Config) -> Result<(), String> {
    let mut router = Router::new();
    router.post("/hook", webhook_handler, "webhook");

    let default_listen = String::from("0.0.0.0:3000");
    let addr_and_port = match config.listen_addr {
        Some(ref addr_and_port) => addr_and_port,
        None => &default_listen,
    };

    match Iron::new(router).http(addr_and_port.as_str()) {
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
