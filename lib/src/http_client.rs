use anyhow::{anyhow, bail};
use maplit::hashmap;
use prometheus::{HistogramTimer, HistogramVec, IntCounterVec};
use reqwest;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;

use crate::errors::*;
use crate::metrics;

pub use reqwest::Response;
pub use reqwest::header::HeaderMap;

pub struct HTTPClient {
    pub api_base: String,
    pub client: reqwest::Client,

    metric_api_responses: Option<IntCounterVec>,
    metric_api_duration: Option<HistogramVec>,
    secret_path: Option<String>,
    retry_rate_limits: bool,
}

const MAX_RATE_LIMIT_RETRIES: u32 = 4;
const MAX_RATE_LIMIT_DELAY: std::time::Duration = std::time::Duration::from_secs(30);

fn retry_after(res: &Response) -> Option<std::time::Duration> {
    res.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

impl HTTPClient {
    pub fn new(api_base: &str) -> Result<HTTPClient> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        Ok(HTTPClient {
            api_base: api_base.into(),
            client,
            metric_api_responses: None,
            metric_api_duration: None,
            secret_path: None,
            retry_rate_limits: false,
        })
    }

    pub fn new_with_headers(api_base: &str, headers: HeaderMap) -> Result<HTTPClient> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .default_headers(headers)
            .build()?;

        Ok(HTTPClient {
            api_base: api_base.into(),
            client,
            metric_api_responses: None,
            metric_api_duration: None,
            secret_path: None,
            retry_rate_limits: false,
        })
    }

    pub fn with_metrics(
        mut self,
        responses: IntCounterVec,
        request_duration: HistogramVec,
    ) -> Self {
        self.metric_api_responses = Some(responses);
        self.metric_api_duration = Some(request_duration);

        self
    }

    pub fn with_secret_path(mut self, path: String) -> Self {
        self.secret_path = Some(path);
        self
    }

    // Retry requests rejected with HTTP 429, honoring any Retry-After header.
    // Intended for APIs with rate limits (e.g. Jira Cloud).
    pub fn with_retry_rate_limits(mut self) -> Self {
        self.retry_rate_limits = true;
        self
    }

    async fn send(&self, mut req: reqwest::RequestBuilder) -> reqwest::Result<Response> {
        let mut attempt: u32 = 0;
        loop {
            let retry_req = if self.retry_rate_limits && attempt < MAX_RATE_LIMIT_RETRIES {
                req.try_clone()
            } else {
                None
            };

            let result = req.send().await;

            match (&result, retry_req) {
                (Ok(res), Some(retry_req))
                    if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS =>
                {
                    attempt += 1;
                    let delay = retry_after(res)
                        .unwrap_or_else(|| std::time::Duration::from_secs(2u64 << attempt))
                        .min(MAX_RATE_LIMIT_DELAY);
                    log::warn!(
                        "Request to {} rate-limited (HTTP 429): retrying in {:?} (attempt {}/{})",
                        res.url(),
                        delay,
                        attempt,
                        MAX_RATE_LIMIT_RETRIES
                    );
                    tokio::time::sleep(delay).await;
                    req = retry_req;
                }
                _ => return result,
            }
        }
    }

    fn make_url(&self, path: &str) -> String {
        if path.is_empty() {
            self.api_base.clone()
        } else if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else if path.starts_with('/') {
            self.api_base.clone() + path
        } else {
            self.api_base.clone() + "/" + path
        }
    }

    pub async fn get_raw(&self, path: &str) -> Result<Response> {
        let _timer = self.maybe_start_timer("get", path);
        let res = self.send(self.client.get(self.make_url(path))).await;
        let res = self.process_resp(res).await?;

        self.maybe_record_ok();
        Ok(res)
    }

    pub async fn get<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let res = self.get_raw(path).await?;
        let res = self.parse_json(res).await?;

        self.maybe_record_ok();
        Ok(res)
    }

    pub async fn post<T, U: Serialize>(&self, path: &str, body: &U) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let _timer = self.maybe_start_timer("post", path);
        let res = self
            .send(self.client.post(self.make_url(path)).json(body))
            .await;
        let res = self.process_resp(res).await?;
        let res = self.parse_json(res).await?;

        self.maybe_record_ok();
        Ok(res)
    }

    pub async fn post_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        let _timer = self.maybe_start_timer("post", path);
        let res = self
            .send(self.client.post(self.make_url(path)).json(body))
            .await;
        self.process_resp(res).await?;

        self.maybe_record_ok();
        Ok(())
    }

    pub async fn post_void_opt<U: Serialize>(&self, path: &str, body: Option<&U>) -> Result<()> {
        let _timer = self.maybe_start_timer("post", path);
        let res = self.client.post(self.make_url(path));
        let res = match body {
            None => res,
            Some(body) => res.json(body),
        };
        let res = self.send(res).await;
        self.process_resp(res).await?;

        self.maybe_record_ok();
        Ok(())
    }

    pub async fn put<T, U: Serialize>(&self, path: &str, body: &U) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let _timer = self.maybe_start_timer("put", path);
        let res = self
            .send(self.client.put(self.make_url(path)).json(body))
            .await;
        let res = self.process_resp(res).await?;
        let res = self.parse_json(res).await?;

        self.maybe_record_ok();
        Ok(res)
    }

    pub async fn put_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        let _timer = self.maybe_start_timer("put", path);
        let res = self
            .send(self.client.put(self.make_url(path)).json(body))
            .await;
        self.process_resp(res).await?;

        self.maybe_record_ok();
        Ok(())
    }

    pub async fn delete_void(&self, path: &str) -> Result<()> {
        let _timer = self.maybe_start_timer("delete", path);
        let res = self.send(self.client.delete(self.make_url(path))).await;
        self.process_resp(res).await?;

        self.maybe_record_ok();
        Ok(())
    }

    fn maybe_record_status(&self, status: &str) {
        if let Some(ref m) = self.metric_api_responses {
            m.with(&hashmap! {"status" => status}).inc();
        }
    }

    fn maybe_record_ok(&self) {
        self.maybe_record_status(reqwest::StatusCode::OK.as_str());
    }

    fn maybe_start_timer(&self, method: &str, path: &str) -> Option<HistogramTimer> {
        self.metric_api_duration.clone().map(|ref m| {
            let path = if self.secret_path.is_some() {
                String::new()
            } else {
                metrics::cleanup_path(path)
            };
            m.with(&hashmap! {
                "method" => method,
                "path" => &path,
            })
            .start_timer()
        })
    }

    fn make_clean_err<T>(&self, e: impl std::error::Error) -> Result<T> {
        let mut msg = format!("{}", e);
        if let Some(ref s) = self.secret_path {
            msg = msg.replace(s, "<redacted>");
        }

        Err(anyhow!("{}", msg))
    }

    async fn process_resp(&self, res: reqwest::Result<Response>) -> Result<Response> {
        let res = match res {
            Ok(r) => r,
            Err(e) => {
                self.maybe_record_status("<unknown>");
                return self.make_clean_err(e);
            }
        };

        match res.error_for_status_ref() {
            Ok(_) => Ok(res),
            Err(e) => {
                self.maybe_record_status(res.status().as_str());
                let err: Result<()> = self.make_clean_err(e);
                let text = res.text().await.unwrap_or_default();
                bail!("{}. Response body: {}", err.unwrap_err(), text);
            }
        }
    }

    pub async fn parse_json<T>(&self, res: Response) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let text = res.text().await.unwrap_or_default();
        log::trace!("Response body: {}", text);

        let result: serde_json::Result<T> = serde_json::from_str(&text);
        match result {
            Ok(r) => Ok(r),
            Err(e) => {
                self.maybe_record_status("<invalid json>");
                let err: Result<()> = self.make_clean_err(e);
                bail!(
                    "Invalid JSON: {}. Response body: {}",
                    err.unwrap_err(),
                    text
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_retry_rate_limited() {
        let mut server = mockito::Server::new_async().await;
        let client = HTTPClient::new(&server.url())
            .unwrap()
            .with_retry_rate_limits();

        // mockito serves the first matching mock that is still below its
        // expected hit count: rate-limit the first request, then succeed.
        let limited = server
            .mock("GET", "/thing")
            .with_status(429)
            .with_header("retry-after", "0")
            .expect(1)
            .create_async()
            .await;
        let ok = server
            .mock("GET", "/thing")
            .with_body(r#"{"value": 42}"#)
            .expect(1)
            .create_async()
            .await;

        let resp = client.get::<serde_json::Value>("/thing").await.unwrap();
        assert_eq!(42, resp["value"].as_u64().unwrap());

        limited.assert_async().await;
        ok.assert_async().await;
    }

    #[tokio::test]
    async fn test_rate_limited_gives_up() {
        let mut server = mockito::Server::new_async().await;
        let client = HTTPClient::new(&server.url())
            .unwrap()
            .with_retry_rate_limits();

        let limited = server
            .mock("GET", "/thing")
            .with_status(429)
            .with_header("retry-after", "0")
            .expect(1 + MAX_RATE_LIMIT_RETRIES as usize)
            .create_async()
            .await;

        let err = client.get::<serde_json::Value>("/thing").await.unwrap_err();
        assert!(err.to_string().contains("429"), "unexpected error: {}", err);

        limited.assert_async().await;
    }

    #[tokio::test]
    async fn test_no_retry_by_default() {
        let mut server = mockito::Server::new_async().await;
        let client = HTTPClient::new(&server.url()).unwrap();

        let limited = server
            .mock("GET", "/thing")
            .with_status(429)
            .with_header("retry-after", "0")
            .expect(1)
            .create_async()
            .await;

        let err = client.get::<serde_json::Value>("/thing").await.unwrap_err();
        assert!(err.to_string().contains("429"), "unexpected error: {}", err);

        limited.assert_async().await;
    }
}
