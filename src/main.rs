extern crate env_logger;
extern crate octobot;
extern crate time;
extern crate thread_id;
extern crate log;

use octobot::config;
use octobot::server;

use env_logger::LogBuilder;
use log::{LogLevelFilter, LogRecord};

fn main() {
    if std::env::args().len() < 2 {
        panic!("Usage: octobot <config-file>")
    }

    setup_logging();

    let config_file = std::env::args().nth(1).unwrap();
    let config = config::parse(&config_file).expect("Error parsing config");

    server::start(config).expect("Failed to start server");
}

fn setup_logging() {
    let formatter = |record: &LogRecord| {
        let t = time::now();
        format!(
            "[{},{:03}][{}:{}] - {} - {}",
            time::strftime("%Y-%m-%d %H:%M:%S", &t).unwrap(),
            t.tm_nsec / 1000_000,
            thread_id::get(),
            std::thread::current().name().unwrap_or(""),
            record.level(),
            record.args()
        )
    };

    let mut builder = LogBuilder::new();
    builder.format(formatter).filter(None, LogLevelFilter::Info);

    let is_info;
    if let Ok(ref env_log) = std::env::var("RUST_LOG") {
        builder.parse(env_log);
        is_info = env_log.is_empty() || env_log.to_lowercase() == "info";
    } else {
        is_info = true;
    }

    if is_info {
        builder.filter(Some("rustls"), LogLevelFilter::Warn);
    }

    builder.init().unwrap();
}
