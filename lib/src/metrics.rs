use prometheus::Registry;
#[cfg(target_os = "linux")]
use prometheus::process_collector::ProcessCollector;
use prometheus::{Gauge, register_gauge_with_registry};
use prometheus::{GaugeVec, register_gauge_vec_with_registry};
use prometheus::{Histogram, register_histogram_with_registry};
use prometheus::{HistogramVec, register_histogram_vec_with_registry};
use prometheus::{IntCounter, register_int_counter_with_registry};
use prometheus::{IntCounterVec, register_int_counter_vec_with_registry};
use std::sync::Arc;
use std::time::Instant;

pub struct Metrics {
    pub registry: Arc<Registry>,

    pub http_duration: HistogramVec,

    pub webhook_events: IntCounterVec,

    pub slack_api_responses: IntCounterVec,
    pub slack_api_duration: HistogramVec,

    pub jira_api_responses: IntCounterVec,
    pub jira_api_duration: HistogramVec,

    pub github_api_responses: IntCounterVec,
    pub github_api_duration: HistogramVec,
    pub github_reviewer_lookup_failures: IntCounter,

    pub current_connection_count: Gauge,
    pub current_webhook_count: Gauge,

    pub current_backport_count: Gauge,
    pub current_force_push_count: Gauge,
    pub current_repo_version_count: Gauge,

    pub jobs_queued: GaugeVec,
    pub jobs_finished: IntCounterVec,

    pub event_bus_published: IntCounterVec,
    pub current_event_stream_count: Gauge,
    pub event_stream_messages: IntCounterVec,
    pub event_stream_ends: IntCounterVec,
    pub event_stream_auth_failures: IntCounterVec,

    pub backport_duration: Histogram,
    pub force_push_duration: Histogram,
    pub repo_version_duration: Histogram,

    pub tokio_running_thread_count: GaugeVec,
    pub tokio_parked_thread_count: GaugeVec,
}

fn http_duration_buckets() -> Vec<f64> {
    vec![0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0]
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

            http_duration: register_histogram_vec_with_registry!(
                "http_requests_duration",
                "Duration of HTTP requests in seconds",
                &["method", "path", "status"],
                http_duration_buckets(),
                registry.as_ref()
            )
            .unwrap(),

            webhook_events: register_int_counter_vec_with_registry!(
                "webhook_events",
                "GitHub webhook deliveries by outcome",
                &["event", "outcome"],
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
                "Duration of jira API requests",
                &["method", "path"],
                http_duration_buckets(),
                registry.as_ref()
            )
            .unwrap(),

            github_api_responses: register_int_counter_vec_with_registry!(
                "github_api_responses",
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

            github_reviewer_lookup_failures: register_int_counter_with_registry!(
                "github_reviewer_lookup_failures",
                "Failed PR reviewer lookups, each notifying without past reviewers",
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

            jobs_queued: register_gauge_vec_with_registry!(
                "jobs_queued",
                "Background jobs not yet started on a job thread",
                &["job"],
                registry.as_ref()
            )
            .unwrap(),

            jobs_finished: register_int_counter_vec_with_registry!(
                "jobs_finished",
                "Background jobs finished, by outcome",
                &["job", "outcome"],
                registry.as_ref()
            )
            .unwrap(),

            event_bus_published: register_int_counter_vec_with_registry!(
                "event_bus_published",
                "Changes published to event stream subscribers",
                &["kind"],
                registry.as_ref()
            )
            .unwrap(),

            current_event_stream_count: register_gauge_with_registry!(
                "current_event_stream_count",
                "The number of open event streams",
                registry.as_ref(),
            )
            .unwrap(),

            event_stream_messages: register_int_counter_vec_with_registry!(
                "event_stream_messages",
                "Messages sent on event streams",
                &["type"],
                registry.as_ref()
            )
            .unwrap(),

            event_stream_ends: register_int_counter_vec_with_registry!(
                "event_stream_ends",
                "Event streams ended, by why they ended",
                &["reason"],
                registry.as_ref()
            )
            .unwrap(),

            event_stream_auth_failures: register_int_counter_vec_with_registry!(
                "event_stream_auth_failures",
                "Refused event stream challenges and subscriptions",
                &["stage", "reason"],
                registry.as_ref()
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

    if !path.starts_with('/') {
        path = "/".to_string() + &path;
    }

    match path.find('.') {
        None => path
            .split('/')
            .filter(|p| p.find('.').is_none())
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

/// Observes a request's duration when dropped. Hyper drops a request whose client hangs up,
/// so one dropped before `respond` is observed with status "cancelled".
pub struct RequestTimer {
    histogram: HistogramVec,
    method: String,
    path: String,
    start: Instant,
    status: Option<String>,
}

impl RequestTimer {
    pub fn start(histogram: &HistogramVec, method: &str, path: &str) -> RequestTimer {
        RequestTimer {
            histogram: histogram.clone(),
            method: method.to_string(),
            path: path.to_string(),
            start: Instant::now(),
            status: None,
        }
    }

    pub fn respond(&mut self, status: &str) {
        self.status = Some(status.to_string());
    }
}

impl Drop for RequestTimer {
    fn drop(&mut self) {
        let status = self.status.as_deref().unwrap_or("cancelled");
        self.histogram
            .with_label_values(&[self.method.as_str(), &self.path, status])
            .observe(self.start.elapsed().as_secs_f64());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(metrics: &Metrics, status: &str) -> u64 {
        metrics
            .http_duration
            .with_label_values(&["GET", "/hooks", status])
            .get_sample_count()
    }

    #[test]
    fn test_request_timer_observes_status() {
        let metrics = Metrics::new();
        let mut timer = RequestTimer::start(&metrics.http_duration, "GET", "/hooks");
        timer.respond("200");
        drop(timer);

        assert_eq!(1, count(&metrics, "200"));
        assert_eq!(0, count(&metrics, "cancelled"));
    }

    #[test]
    fn test_request_dropped_without_response_is_cancelled() {
        let metrics = Metrics::new();
        drop(RequestTimer::start(&metrics.http_duration, "GET", "/hooks"));

        assert_eq!(1, count(&metrics, "cancelled"));
    }
}
