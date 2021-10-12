use prometheus::process_collector::ProcessCollector;
use prometheus::Registry;
use prometheus::{register_gauge_vec_with_registry, GaugeVec};
use prometheus::{register_gauge_with_registry, Gauge};
use prometheus::{register_histogram_vec_with_registry, HistogramVec};
use prometheus::{register_histogram_with_registry, Histogram};
use prometheus::{register_int_counter_vec_with_registry, IntCounterVec};
use std::sync::Arc;

pub struct Metrics {
    pub registry: Arc<Registry>,

    pub http_responses: IntCounterVec,
    pub http_duration: HistogramVec,

    pub slack_api_responses: IntCounterVec,
    pub slack_api_duration: HistogramVec,

    pub jira_api_responses: IntCounterVec,
    pub jira_api_duration: HistogramVec,

    pub github_api_responses: IntCounterVec,
    pub github_api_duration: HistogramVec,

    pub current_connection_count: Gauge,
    pub current_webhook_count: Gauge,

    pub current_backport_count: Gauge,
    pub current_force_push_count: Gauge,
    pub current_repo_version_count: Gauge,

    pub backport_duration: Histogram,
    pub force_push_duration: Histogram,
    pub repo_version_duration: Histogram,

    pub tokio_running_thread_count: GaugeVec,
    pub tokio_parked_thread_count: GaugeVec,
}

fn http_duration_buckets() -> Vec<f64> {
    vec![0.005, 0.1, 0.5, 1.0, 2.0, 10.0, 30.0, 60.0]
}

fn job_duration_buckets() -> Vec<f64> {
    vec![0.1, 0.5, 1.0, 2.0, 10.0, 30.0, 60.0, 300.0]
}

impl Metrics {
    pub fn new() -> Arc<Metrics> {
        let registry = Arc::new(
            Registry::new_custom(Some("octobot".into()), None).expect("create prometheus registry"),
        );
        Metrics::register_default(&registry);

        Arc::new(Metrics {
            registry: registry.clone(),

            http_responses: register_int_counter_vec_with_registry!(
                "http_responses",
                "HTTP response codes",
                &["status"],
                registry.as_ref()
            )
            .unwrap(),

            http_duration: register_histogram_vec_with_registry!(
                "http_requests_duration",
                "Duration of HTTP requests in seconds",
                &["method", "path"],
                http_duration_buckets(),
                registry.as_ref()
            )
            .unwrap(),

            slack_api_responses: register_int_counter_vec_with_registry!(
                "slack_api_responses",
                "Slack API responses",
                &["status"],
                registry.as_ref()
            )
            .unwrap(),

            slack_api_duration: register_histogram_vec_with_registry!(
                "slack_api_request_duration",
                "Duration of slack API requests",
                &["method", "path"],
                http_duration_buckets(),
                registry.as_ref()
            )
            .unwrap(),

            jira_api_responses: register_int_counter_vec_with_registry!(
                "jira_api_responses",
                "jira API responses",
                &["status"],
                registry.as_ref()
            )
            .unwrap(),

            jira_api_duration: register_histogram_vec_with_registry!(
                "jira_api_request_duration",
                "jira API responses",
                &["method", "path"],
                http_duration_buckets(),
                registry.as_ref()
            )
            .unwrap(),

            github_api_responses: register_int_counter_vec_with_registry!(
                "gihtub_api_responses",
                "Github API responses",
                &["status"],
                registry.as_ref()
            )
            .unwrap(),

            github_api_duration: register_histogram_vec_with_registry!(
                "github_api_request_duration",
                "Duration of github API requests",
                &["method", "path"],
                http_duration_buckets(),
                registry.as_ref()
            )
            .unwrap(),

            current_connection_count: register_gauge_with_registry!(
                "current_connection_count",
                "The number of current http connections",
                registry.as_ref(),
            )
            .unwrap(),

            current_webhook_count: register_gauge_with_registry!(
                "current_webhook_count",
                "The number of current webhooks being handled",
                registry.as_ref(),
            )
            .unwrap(),

            current_backport_count: register_gauge_with_registry!(
                "current_backport_count",
                "The number of current backport jobs being run",
                registry.as_ref(),
            )
            .unwrap(),

            current_force_push_count: register_gauge_with_registry!(
                "current_force_push_count",
                "The number of current force-push jobs being run",
                registry.as_ref(),
            )
            .unwrap(),

            current_repo_version_count: register_gauge_with_registry!(
                "current_repo_version_count",
                "The number of current repo version jobs being run",
                registry.as_ref(),
            )
            .unwrap(),

            backport_duration: register_histogram_with_registry!(
                "backport_duration",
                "Duration of backport job",
                job_duration_buckets(),
                registry.as_ref(),
            )
            .unwrap(),

            force_push_duration: register_histogram_with_registry!(
                "force_push_duration",
                "Duration of force-push job",
                job_duration_buckets(),
                registry.as_ref(),
            )
            .unwrap(),

            repo_version_duration: register_histogram_with_registry!(
                "repo_version_duration",
                "Duration of repo-version job",
                job_duration_buckets(),
                registry.as_ref(),
            )
            .unwrap(),

            tokio_running_thread_count: register_gauge_vec_with_registry!(
                "tokio_running_thread_count",
                "Tokio running thread counts per runtime",
                &["runtime"],
                registry.as_ref()
            )
            .unwrap(),

            tokio_parked_thread_count: register_gauge_vec_with_registry!(
                "tokio_parked_thread_count",
                "Tokio parked thread counts per runtime",
                &["runtime"],
                registry.as_ref()
            )
            .unwrap(),
        })
    }

    #[cfg(target_os = "linux")]
    fn register_default(reg: &Registry) {
        if let Err(e) = reg.register(Box::new(ProcessCollector::for_self())) {
            log::error!("Failed to register process metrics collector: {}", e);
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn register_default(_: &Registry) {
        log::info!("No process collection metrics are available for this platform");
    }
}

pub fn cleanup_path(path: &str) -> String {
    let mut path = path.to_string();
    if path.is_empty() {
        return path;
    }

    if !path.starts_with("/") {
        path = "/".to_string() + &path;
    }

    match path.find(".") {
        None => path
            .split("/")
            .filter(|p| p.find(".").is_none())
            .take(3)
            .collect::<Vec<_>>()
            .join("/"),
        Some(_) => "<static>".to_string(),
    }
}

pub struct ScopedGaugeCounter {
    gauge: Gauge,
}

pub fn scoped_inc(gauge: &Gauge) -> ScopedGaugeCounter {
    let scoped = ScopedGaugeCounter {
        gauge: gauge.clone(),
    };
    scoped.gauge.inc();
    scoped
}

impl Drop for ScopedGaugeCounter {
    fn drop(&mut self) {
        self.gauge.dec()
    }
}
