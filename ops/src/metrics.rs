use prometheus::process_collector::ProcessCollector;
use std::sync::Arc;

const NAMESPACE: &'static str = "octobot";

pub struct Registry {
    pub registry: Arc<prometheus::Registry>,
}

impl Registry {
    pub fn new() -> Arc<Registry> {
        let registry = Arc::new(Registry {
            registry: Arc::new(prometheus::Registry::new()),
        });

        registry.register_default();
        registry
    }

    #[cfg(target_os = "linux")]
    fn register_default(&self) {
        let pid = unsafe { libc::getpid() };
        if let Err(e) = self.registry.register(Box::new(ProcessCollector::new(pid, NAMESPACE))) {
            log::error!("Failed to register process metrics collector: {}", e);
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn register_default(&self) {
        log::info!("No process collection metrics are available for this platform");
    }
}
