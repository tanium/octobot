use std;

use futures::{Future, future};
use tokio;
use tokio::executor::thread_pool;

pub fn new(num_threads: usize, name: &str) -> tokio::runtime::Runtime {
    let mut threadpool = thread_pool::Builder::new();
    threadpool.name_prefix(format!("{}-", name)).pool_size(num_threads);

    tokio::runtime::Builder::new().threadpool_builder(threadpool).build().unwrap()
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
