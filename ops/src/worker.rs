use std::sync::{Arc, Mutex};

use tokio;

pub trait Worker<T: Send + 'static>: Send + Sync {
    fn send(&self, req: T);
}

#[async_trait::async_trait]
pub trait Runner<T: Send + 'static>: Send + Sync {
    async fn handle(&self, req: T);
}

pub struct TokioWorker<T: Send + Sync + 'static> {
    runner: Arc<dyn Runner<T>>,
    runtime: Arc<Mutex<tokio::runtime::Runtime>>,
}

impl<T: Send + Sync + 'static> TokioWorker<T> {
    pub fn new(
        runtime: Arc<Mutex<tokio::runtime::Runtime>>,
        runner: Arc<dyn Runner<T>>,
    ) -> Arc<dyn Worker<T>> {
        Arc::new(TokioWorker { runner, runtime })
    }
}

impl<T: Send + Sync + 'static> Worker<T> for TokioWorker<T> {
    fn send(&self, req: T) -> () {
        let runner = self.runner.clone();
        self.runtime
            .lock()
            .unwrap()
            .spawn(async move { runner.handle(req).await });
    }
}
