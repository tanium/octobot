use prometheus::process_collector::ProcessCollector;
use std::sync::Arc;

pub struct Registry {
    pub registry: Arc<prometheus::Registry>,
}

impl Registry {
    pub fn new() -> Arc<Registry> {
        let registry = Arc::new(Registry {
            registry: Arc::new(prometheus::Registry::new_custom(Some("octobot".into()), None).expect("create prometheus registry")),
        });

        registry.register_default();
        registry
    }

    #[cfg(target_os = "linux")]
    fn register_default(&self) {
        if let Err(e) = self.registry.register(Box::new(ProcessCollector::for_self())) {
            log::error!("Failed to register process metrics collector: {}", e);
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn register_default(&self) {
        log::info!("No process collection metrics are available for this platform");
    }
}
