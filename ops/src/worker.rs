use std::sync::Arc;
use std::time::Duration;

use log::error;
use tokio;
use tokio_util::task::TaskTracker;

use octobot_lib::errors::Result;
use octobot_lib::metrics::Metrics;

pub trait Worker<T: Send + 'static>: Send + Sync {
    fn send(&self, req: T);
}

#[async_trait::async_trait]
pub trait Runner<T: Send + 'static>: Send + Sync {
    /// An error fails the job; the worker logs and counts it.
    async fn handle(&self, req: T) -> Result<()>;
}

/// The runtime that worker jobs run on, tracked so that shutdown can wait for them.
/// Must not be dropped from within an async context, as it owns a tokio runtime.
pub struct Jobs {
    runtime: tokio::runtime::Runtime,
    tracker: TaskTracker,
    metrics: Arc<Metrics>,
}

impl Jobs {
    pub fn new(runtime: tokio::runtime::Runtime, metrics: Arc<Metrics>) -> Arc<Jobs> {
        Arc::new(Jobs {
            runtime,
            tracker: TaskTracker::new(),
            metrics,
        })
    }

    fn spawn<F>(&self, job: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.tracker.spawn_on(job, self.runtime.handle());
    }

    /// Waits for running jobs, including jobs they send to other workers.
    /// Returns how many jobs were still running at the deadline.
    pub async fn shutdown(&self, deadline: Duration) -> usize {
        // A closed tracker still tracks new jobs; it only lets wait() finish once none are left.
        self.tracker.close();
        let _ = tokio::time::timeout(deadline, self.tracker.wait()).await;
        self.tracker.len()
    }
}

pub struct TokioWorker<T: Send + Sync + 'static> {
    /// The job label in metrics
    name: &'static str,
    runner: Arc<dyn Runner<T>>,
    jobs: Arc<Jobs>,
}

impl<T: Send + Sync + 'static> TokioWorker<T> {
    pub fn new_worker(
        jobs: Arc<Jobs>,
        name: &'static str,
        runner: Arc<dyn Runner<T>>,
    ) -> Arc<dyn Worker<T>> {
        Arc::new(TokioWorker { name, runner, jobs })
    }
}

impl<T: Send + Sync + 'static> Worker<T> for TokioWorker<T> {
    fn send(&self, req: T) {
        let name = self.name;
        let runner = self.runner.clone();
        let metrics = self.jobs.metrics.clone();
        // Queued until a job thread first runs it. Git runs block the threads they run on, so
        // jobs wait behind them. A started job waiting again is not counted.
        let queued = metrics.jobs_queued.with_label_values(&[name]);
        queued.inc();
        self.jobs.spawn(async move {
            queued.dec();
            let outcome = match runner.handle(req).await {
                Ok(()) => "success",
                Err(e) => {
                    error!("{} job failed: {:#}", name, e);
                    "failure"
                }
            };
            metrics
                .jobs_finished
                .with_label_values(&[name, outcome])
                .inc();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::task::{Context, Waker};
    use tokio::sync::oneshot;

    // The jobs runtime can't be dropped inside an async test, so tests drive shutdown from a
    // separate runtime. Paused time fires a deadline as soon as that runtime is idle, even while
    // a job is still finishing on the jobs runtime, so only the deadline test pauses it.
    fn new_jobs(paused: bool) -> (Arc<Jobs>, tokio::runtime::Runtime) {
        let jobs_runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let test_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(paused)
            .build()
            .unwrap();
        (Jobs::new(jobs_runtime, Metrics::new()), test_runtime)
    }

    fn is_pending<F: Future>(test_runtime: &tokio::runtime::Runtime, fut: Pin<&mut F>) -> bool {
        let _guard = test_runtime.enter();
        fut.poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    }

    struct Gated {
        gate: Mutex<Option<oneshot::Receiver<()>>>,
        done: Mutex<Vec<&'static str>>,
    }

    #[async_trait::async_trait]
    impl Runner<&'static str> for Gated {
        async fn handle(&self, name: &'static str) -> Result<()> {
            let gate = self.gate.lock().unwrap().take();
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            self.done.lock().unwrap().push(name);
            Ok(())
        }
    }

    fn gated() -> (Arc<Gated>, oneshot::Sender<()>) {
        let (tx, rx) = oneshot::channel();
        let runner = Arc::new(Gated {
            gate: Mutex::new(Some(rx)),
            done: Mutex::new(vec![]),
        });
        (runner, tx)
    }

    #[test]
    fn test_shutdown_waits_for_running_job() {
        let (jobs, test_runtime) = new_jobs(false);
        let (runner, release) = gated();
        let worker = TokioWorker::new_worker(jobs.clone(), "gated", runner.clone());

        worker.send("job");

        let mut shutdown = Box::pin(jobs.shutdown(Duration::from_secs(3600)));
        assert!(is_pending(&test_runtime, shutdown.as_mut()));

        release.send(()).unwrap();
        assert_eq!(0, test_runtime.block_on(shutdown));
        assert_eq!(vec!["job"], *runner.done.lock().unwrap());
    }

    struct Forward {
        next: Arc<dyn Worker<&'static str>>,
    }

    #[async_trait::async_trait]
    impl Runner<&'static str> for Forward {
        async fn handle(&self, name: &'static str) -> Result<()> {
            self.next.send(name);
            Ok(())
        }
    }

    #[test]
    fn test_shutdown_waits_for_jobs_sent_by_jobs() {
        let (jobs, test_runtime) = new_jobs(false);
        let (runner, release) = gated();
        let gated_worker = TokioWorker::new_worker(jobs.clone(), "gated", runner.clone());
        let forward = TokioWorker::new_worker(
            jobs.clone(),
            "forward",
            Arc::new(Forward {
                next: gated_worker.clone(),
            }),
        );

        let mut shutdown = Box::pin(jobs.shutdown(Duration::from_secs(3600)));
        forward.send("forwarded");
        assert!(is_pending(&test_runtime, shutdown.as_mut()));

        release.send(()).unwrap();
        assert_eq!(0, test_runtime.block_on(shutdown));
        assert_eq!(vec!["forwarded"], *runner.done.lock().unwrap());
    }

    #[test]
    fn test_shutdown_gives_up_at_deadline() {
        let (jobs, test_runtime) = new_jobs(true);
        let (runner, _release) = gated();
        let worker = TokioWorker::new_worker(jobs.clone(), "gated", runner.clone());

        worker.send("stuck");

        assert_eq!(
            1,
            test_runtime.block_on(jobs.shutdown(Duration::from_secs(30)))
        );
        assert!(runner.done.lock().unwrap().is_empty());
    }

    /// Blocks its job thread until released, the way a git run does.
    struct Blocking {
        entered: Mutex<std::sync::mpsc::Sender<()>>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    #[async_trait::async_trait]
    impl Runner<&'static str> for Blocking {
        async fn handle(&self, _name: &'static str) -> Result<()> {
            self.entered.lock().unwrap().send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(())
        }
    }

    #[test]
    fn test_jobs_waiting_for_a_thread_are_queued() {
        let (jobs, test_runtime) = new_jobs(false);
        let (entered_tx, entered) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let runner = Arc::new(Blocking {
            entered: Mutex::new(entered_tx),
            release: Mutex::new(release_rx),
        });
        let worker = TokioWorker::new_worker(jobs.clone(), "blocking", runner);
        let metrics = jobs.metrics.clone();
        let queued = metrics.jobs_queued.with_label_values(&["blocking"]);

        worker.send("first");
        entered.recv().unwrap();
        // The jobs runtime's one thread is blocked by the first job.
        worker.send("second");
        assert_eq!(1.0, queued.get());

        release.send(()).unwrap();
        release.send(()).unwrap();
        assert_eq!(
            0,
            test_runtime.block_on(jobs.shutdown(Duration::from_secs(3600)))
        );
        assert_eq!(0.0, queued.get());
        let succeeded = metrics
            .jobs_finished
            .with_label_values(&["blocking", "success"]);
        assert_eq!(2, succeeded.get());
    }

    struct Failing;

    #[async_trait::async_trait]
    impl Runner<&'static str> for Failing {
        async fn handle(&self, _name: &'static str) -> Result<()> {
            Err(anyhow::anyhow!("failed"))
        }
    }

    #[test]
    fn test_failed_jobs_are_counted() {
        let (jobs, test_runtime) = new_jobs(false);
        let worker = TokioWorker::new_worker(jobs.clone(), "failing", Arc::new(Failing));

        worker.send("job");
        test_runtime.block_on(jobs.shutdown(Duration::from_secs(3600)));

        let failed = jobs
            .metrics
            .jobs_finished
            .with_label_values(&["failing", "failure"]);
        assert_eq!(1, failed.get());
    }
}
