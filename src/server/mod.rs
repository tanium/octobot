extern crate iron;
extern crate router;

mod http;
mod handlers;

pub fn start(addr_and_port: &str) -> Result<(), String> {
    http::start(addr_and_port)
}
