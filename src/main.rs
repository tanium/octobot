extern crate env_logger;
extern crate logger;
extern crate ring;
extern crate rustc_serialize;
extern crate serde_json;
extern crate toml;

#[macro_use]
extern crate log;

mod config;
mod git;
mod messenger;
mod server;
mod util;

fn main() {
    if std::env::args().len() < 2 {
        panic!("Usage: octobot <config-file>")
    }

    env_logger::init().unwrap();

    let config = config::parse(std::env::args().nth(1).unwrap()).expect("Error parsing config");

    server::start(config).expect("Failed to start server");
}


