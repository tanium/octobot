use prometheus::Registry;
use prometheus::process_collector::ProcessCollector;
use std::sync::Arc;

pub struct Metrics {
    pub registry: Arc<Registry>,
}

impl Metrics{
    pub fn new() -> Arc<Metrics> {
        let metrics = Arc::new(Metrics {
            registry: Arc::new(Registry::new_custom(Some("octobot".into()), None).expect("create prometheus registry")),
        });

        metrics.register_default();
        metrics
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
