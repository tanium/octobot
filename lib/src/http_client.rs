use failure::bail;
use maplit::hashmap;
use prometheus::{HistogramTimer, HistogramVec, IntCounterVec};
use reqwest;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;

use crate::errors::*;
use crate::metrics;

pub use reqwest::header::HeaderMap;
pub use reqwest::Response;

pub struct HTTPClient {
    pub api_base: String,
    pub client: reqwest::Client,

    metric_api_responses: Option<IntCounterVec>,
    metric_api_duration: Option<HistogramVec>,
    secret_path: Option<String>,
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

    pub async fn get<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let _timer = self.maybe_start_timer("get", path);
        let res = self.client.get(&self.make_url(path)).send().await;
        let res = self.process_resp(res).await?;
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
            .client
            .post(&self.make_url(path))
            .json(body)
            .send()
            .await;
        let res = self.process_resp(res).await?;
        let res = self.parse_json(res).await?;

        self.maybe_record_ok();
        Ok(res)
    }

    pub async fn post_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        let _timer = self.maybe_start_timer("post", path);
        let res = self
            .client
            .post(&self.make_url(path))
            .json(body)
            .send()
            .await;
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
            .client
            .put(&self.make_url(path))
            .json(body)
            .send()
            .await;
        let res = self.process_resp(res).await?;
        let res = self.parse_json(res).await?;

        self.maybe_record_ok();
        Ok(res)
    }

    pub async fn put_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        let _timer = self.maybe_start_timer("put", path);
        let res = self
            .client
            .put(&self.make_url(path))
            .json(body)
            .send()
            .await;
        self.process_resp(res).await?;

        self.maybe_record_ok();
        Ok(())
    }

    pub async fn delete_void(&self, path: &str) -> Result<()> {
        let _timer = self.maybe_start_timer("delete", path);
        let res = self.client.delete(&self.make_url(path)).send().await;
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

    fn make_clean_err<T>(&self, e: impl failure::Fail) -> Result<T> {
        if let Some(ref s) = self.secret_path {
            let msg = format!("{}", e);
            bail!("{}", msg.replace(s, "<redacted>"));
        } else {
            bail!(e);
        }
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

    async fn parse_json<T>(&self, res: Response) -> Result<T>
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
                self.make_clean_err(e)
            }
        }
    }
}
