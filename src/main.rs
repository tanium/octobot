extern crate rustc_serialize;
extern crate toml;

mod server;
mod config;

fn main() {
    if std::env::args().len() < 2 {
        panic!("Usage: octobot <config-file>")
    }

    let config = config::parse(std::env::args().nth(1).unwrap()).expect("Error parsing config");

    server::start(config).expect("Failed to start server");
}


