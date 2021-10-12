use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio;

use octobot_lib::metrics::Metrics;

pub fn new(
    num_threads: usize,
    name: &'static str,
    metrics: Arc<Metrics>,
) -> tokio::runtime::Runtime {
    let running_count = metrics
        .tokio_running_thread_count
        .with_label_values(&[name]);
    let parked_count = metrics.tokio_parked_thread_count.with_label_values(&[name]);

    tokio::runtime::Builder::new_multi_thread()
        .thread_name_fn(move || {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            format!("{}-{}", name, id)
        })
        .worker_threads(num_threads)
        .enable_all()
        .on_thread_start({
            let running_count = running_count.clone();
            move || {
                running_count.inc();
            }
        })
        .on_thread_stop({
            let running_count = running_count.clone();
            move || {
                running_count.dec();
            }
        })
        .on_thread_park({
            let parked_count = parked_count.clone();
            move || {
                parked_count.inc();
            }
        })
        .on_thread_unpark({
            let parked_count = parked_count.clone();
            move || {
                parked_count.dec();
            }
        })
        .build()
        .unwrap()
}

pub fn run<T>(num_threads: usize, metrics: Arc<Metrics>, fut: T) -> ()
where
    T: std::future::Future + Send + 'static,
    T::Output: Send + 'static,
{
    // need at least two threads or it can get stuck on startup.
    let num_threads = std::cmp::max(2, num_threads);

    let runtime = self::new(num_threads, "runtime", metrics);

    runtime.block_on(fut);
}
