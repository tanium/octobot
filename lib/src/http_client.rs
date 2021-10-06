use failure::format_err;
use reqwest;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;

use crate::errors::*;

pub use reqwest::header::HeaderMap;

pub struct HTTPClient {
    pub api_base: String,
    pub client: reqwest::Client,
}

impl HTTPClient {
    pub fn new(api_base: &str) -> Result<HTTPClient> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        Ok(HTTPClient {
            api_base: api_base.into(),
            client,
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
        })
    }

    fn make_url(&self, path: &str) -> String {
        if path.is_empty() {
            self.api_base.clone()
        } else if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else if path.starts_with("/") {
            self.api_base.clone() + path
        } else {
            self.api_base.clone() + "/" + path
        }
    }

    pub async fn get<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.client
            .get(&self.make_url(path))
            .send()
            .await?
            .error_for_status()?
            .json::<T>()
            .await
            .map_err(|e| format_err!("{}", e))
    }

    pub async fn post<T, U: Serialize>(&self, path: &str, body: &U) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.client
            .post(&self.make_url(path))
            .json(body)
            .send()
            .await?
            .error_for_status()?
            .json::<T>()
            .await
            .map_err(|e| format_err!("{}", e))
    }

    pub async fn post_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        self.client
            .post(&self.make_url(path))
            .json(body)
            .send()
            .await
            .map_err(|e| format_err!("{}", e))?
            .error_for_status()?;
        Ok(())
    }

    pub async fn put<T, U: Serialize>(&self, path: &str, body: &U) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.client
            .put(&self.make_url(path))
            .json(body)
            .send()
            .await?
            .error_for_status()?
            .json::<T>()
            .await
            .map_err(|e| format_err!("{}", e))
    }

    pub async fn put_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        self.client
            .put(&self.make_url(path))
            .json(body)
            .send()
            .await
            .map_err(|e| format_err!("{}", e))?
            .error_for_status()?;
        Ok(())
    }

    pub async fn delete_void(&self, path: &str) -> Result<()> {
        self.client
            .delete(&self.make_url(path))
            .send()
            .await
            .map_err(|e| format_err!("{}", e))?
            .error_for_status()?;
        Ok(())
    }
}
