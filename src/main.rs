extern crate env_logger;
extern crate octobot;
extern crate time;
extern crate thread_id;

#[macro_use]
extern crate log;

use octobot::config;
use octobot::server;

use log::{LogRecord, LogLevelFilter};
use env_logger::LogBuilder;

fn main() {
    if std::env::args().len() < 2 {
        panic!("Usage: octobot <config-file>")
    }

    setup_logging();

    let config = config::parse(std::env::args().nth(1).unwrap()).expect("Error parsing config");

    server::start(config).expect("Failed to start server");
}

fn setup_logging() {
    let formatter = |record: &LogRecord| {
        let t = time::now();
        format!("[{},{:03}][{}:{}] - {} - {}",
                time::strftime("%Y-%m-%d %H:%M:%S", &t).unwrap(),
                t.tm_nsec / 1000_000,
                thread_id::get(),
                std::thread::current().name().unwrap_or(""),
                record.level(),
                record.args())
    };

    let mut builder = LogBuilder::new();
    builder.format(formatter).filter(None, LogLevelFilter::Info);
    if std::env::var("RUST_LOG").is_ok() {
        builder.parse(&std::env::var("RUST_LOG").unwrap());
    }

    builder.init().unwrap();
}
