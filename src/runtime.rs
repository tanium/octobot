use std;

use futures::{future, Future};
use tokio;

pub fn new(num_threads: usize, name: &str) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new()
        .name_prefix(format!("{}-", name))
        .blocking_threads(num_threads)
        .build()
        .unwrap()
}

pub fn run<F>(num_threads: usize, func: F) -> ()
where
    F: FnOnce() -> () + Send + 'static,
{
    // need at least two threads or it can get stuck on startup.
    let num_threads = std::cmp::max(2, num_threads);

    let mut runtime = self::new(num_threads, "runtime");

    runtime.spawn(future::lazy(move || {
        func();
        future::ok(())
    }));

    runtime.shutdown_on_idle().wait().unwrap();
}
