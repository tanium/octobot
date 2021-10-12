use std;

use tokio;

pub fn new(num_threads: usize, name: &str) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .thread_name(format!("{}-", name))
        .worker_threads(num_threads)
        .enable_all()
        .build()
        .unwrap()
}

pub fn run<T>(num_threads: usize, fut: T) -> ()
where
    T: std::future::Future + Send + 'static,
    T::Output: Send + 'static,
{
    // need at least two threads or it can get stuck on startup.
    let num_threads = std::cmp::max(2, num_threads);

    let runtime = self::new(num_threads, "runtime");

    runtime.block_on(fut);
}
