use std::sync::Arc;

use hyper::{Body, Request, Response, HeaderMap, StatusCode};

use octobot_lib::metrics::Metrics;
use octobot_lib::errors::Result;
use octobot_lib::config::Config;
use octobot_lib::passwd;

use crate::server::http::Handler;
use crate::http_util;

pub struct MetricsScrapeHandler {
    config: Arc<Config>,
    registry: Arc<Metrics>,
}

impl MetricsScrapeHandler {
    pub fn new(config: Arc<Config>, registry: Arc<Metrics>) -> Box<MetricsScrapeHandler> {
        Box::new(MetricsScrapeHandler {
            config,
            registry,
        })
    }

    fn validate(&self, headers: &HeaderMap) -> Option<(StatusCode, String)> {
        if http_util::is_dev_mode() {
            return None;
        }

        let parts = match headers.get(hyper::header::AUTHORIZATION) {
            Some(v) => match v.to_str() {
                Ok(v) => v.splitn(2, " ").collect::<Vec<_>>(),
                Err(e) => return Some((StatusCode::BAD_REQUEST, format!("Invalid authorization header: {}", e))),
            },
            None => return Some((StatusCode::UNAUTHORIZED, "No authorization header".into())),
        };

        if parts.len() != 2 || (parts[0] != "Bearer" && parts[0] != "Token") {
            return Some((StatusCode::BAD_REQUEST, "Invalid authorization header type".into()));
        }

        if let Some(config) = &self.config.metrics {
            if passwd::verify_password(&parts[1], &config.salt, &config.pass_hash) {
                return None
            } else {
                return Some((StatusCode::FORBIDDEN, "Invalid password".into()));
            }
        } else {
            return Some((StatusCode::FORBIDDEN, "No metrics authorization configured".into()));
        }
    }
}

#[async_trait::async_trait]
impl Handler for MetricsScrapeHandler {
    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>> {
        if let Some((err, msg)) = self.validate(req.headers()) {
            return Ok(http_util::new_msg_resp(err, msg));
        }

        let metrics = self.registry.registry.gather();
        let text = prometheus::TextEncoder::new().encode_to_string(&metrics)?;
        Ok(http_util::new_msg_resp(hyper::StatusCode::OK, text))
    }
}
