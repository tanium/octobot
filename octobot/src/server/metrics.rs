use std::sync::Arc;

use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::{HeaderMap, Request, Response, StatusCode};

use octobot_lib::config::Config;
use octobot_lib::errors::Result;
use octobot_lib::metrics::Metrics;
use octobot_lib::passwd;

use crate::http_util::{self, Body};
use crate::server::http::Handler;

pub struct MetricsScrapeHandler {
    config: Arc<Config>,
    registry: Arc<Metrics>,
}

impl MetricsScrapeHandler {
    pub fn new(config: Arc<Config>, registry: Arc<Metrics>) -> Box<MetricsScrapeHandler> {
        Box::new(MetricsScrapeHandler { config, registry })
    }

    fn validate(&self, headers: &HeaderMap) -> Option<(StatusCode, String)> {
        if http_util::is_dev_mode() {
            return None;
        }

        let parts = match headers.get(hyper::header::AUTHORIZATION) {
            Some(v) => match v.to_str() {
                Ok(v) => v.splitn(2, ' ').collect::<Vec<_>>(),
                Err(e) => {
                    return Some((
                        StatusCode::BAD_REQUEST,
                        format!("Invalid authorization header: {}", e),
                    ));
                }
            },
            None => return Some((StatusCode::UNAUTHORIZED, "No authorization header".into())),
        };

        if parts.len() != 2 || (parts[0] != "Bearer" && parts[0] != "Token") {
            return Some((
                StatusCode::BAD_REQUEST,
                "Invalid authorization header type".into(),
            ));
        }

        if let Some(config) = &self.config.metrics {
            if passwd::verify_password(parts[1], &config.salt, &config.pass_hash) {
                None
            } else {
                Some((StatusCode::FORBIDDEN, "Invalid password".into()))
            }
        } else {
            Some((
                StatusCode::FORBIDDEN,
                "No metrics authorization configured".into(),
            ))
        }
    }
}

#[async_trait::async_trait]
impl Handler for MetricsScrapeHandler {
    async fn handle(&self, req: Request<Incoming>) -> Result<Response<Body>> {
        if let Some((err, msg)) = self.validate(req.headers()) {
            return Ok(http_util::new_msg_resp(err, msg));
        }

        scrape_resp(&self.registry)
    }
}

fn scrape_resp(registry: &Metrics) -> Result<Response<Body>> {
    let metrics = registry.registry.gather();
    let text = prometheus::TextEncoder::new().encode_to_string(&metrics)?;
    let mut resp = http_util::new_msg_resp(hyper::StatusCode::OK, text);
    resp.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        HeaderValue::from_static(prometheus::TEXT_FORMAT),
    );
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrape_content_type() {
        let resp = scrape_resp(&Metrics::new()).unwrap();
        assert_eq!(
            resp.headers().get(hyper::header::CONTENT_TYPE).unwrap(),
            "text/plain; version=0.0.4"
        );
    }
}
