use std::collections::HashMap;
use futures::future::Future;
use futures::Stream;
use hyper;
use hyper_rustls::HttpsConnector;
use hyper::header::{UserAgent};
use hyper::{Method, Request};
use serde_json;
use serde::ser::Serialize;
use serde::de::DeserializeOwned;
use tokio_core::reactor::Core;

pub struct HTTPClient {
    api_base: String,
    headers: HashMap<&'static str, String>,
}

struct InternalResp {
    data: Vec<u8>,
}

impl HTTPClient {
    pub fn new(api_base: &str) -> HTTPClient {
        HTTPClient {
            api_base: api_base.into(),
            headers: HashMap::new(),
        }
    }

    pub fn with_headers(self, headers: HashMap<&'static str, String>) -> HTTPClient {
        let mut c = self;
        c.headers = headers;
        c
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.request_de::<T, ()>(Method::Get, path, None)
    }

    pub fn post<T: DeserializeOwned, U: Serialize>(&self, path: &str, body: &U) -> Result<T, String> {
        self.request_de::<T, U>(Method::Post, path, Some(body))
    }

    pub fn post_void<U: Serialize>(&self, path: &str, body: &U) -> Result<(), String> {
        self.request_void::<U>(Method::Post, path, Some(body))
    }

    pub fn put<T: DeserializeOwned, U: Serialize>(&self, path: &str, body: &U) -> Result<T, String> {
        self.request_de::<T, U>(Method::Put, path, Some(body))
    }

    pub fn put_void<U: Serialize>(&self, path: &str, body: &U) -> Result<(), String> {
        self.request_void::<U>(Method::Put, path, Some(body))
    }

    pub fn delete_void(&self, path: &str) -> Result<(), String> {
        self.request_void::<()>(Method::Delete, path, None)
    }

    fn request_de<T: DeserializeOwned, U: Serialize>(&self, method: Method, path: &str, body: Option<&U>)
            -> Result<T, String> {
        let res = self.request(method, path, body)?;

        let obj: T = match serde_json::from_slice(&res.data) {
            Ok(obj) => obj,
            Err(e) => return Err(format!("Could not parse response: {}\n---\n{}\n---", e, String::from_utf8_lossy(&res.data))),
        };

        Ok(obj)
    }

    fn request_void<U: Serialize>(&self, method: Method, path: &str, body: Option<&U>)
            -> Result<(), String> {
        self.request(method, path, body)?;
        Ok(())
    }

    fn request<U: Serialize>(&self, method: Method, path: &str, body: Option<&U>)
                                             -> Result<InternalResp, String> {
        let url;
        if path.is_empty() {
            url = self.api_base.clone();
        } else if path.starts_with("http://") || path.starts_with("https://") {
            url = path.to_string();
        } else if path.starts_with("/") {
            url = self.api_base.clone() + path;
        } else {
            url = self.api_base.clone() + "/" + path;
        }
        let url = match url.parse() {
            Ok(u) => u,
            Err(e) => return Err(format!("Error: {}", e)),
        };

        let body_json: String;

        let mut core = match Core::new() {
            Ok(c) => c,
            Err(e) => return Err(format!("Error constructing Core: {}", e)),
        };

        // TODO: I wonder if these objects are expensive to create and we should be sharing them across requests?
        let https = HttpsConnector::new(4, &core.handle());
        let client = hyper::Client::configure().connector(https).build(&core.handle());
        let mut req = Request::new(method, url);
        req.headers_mut().set(UserAgent::new("octobot"));

        for (k, v) in &self.headers {
           req.headers_mut().set_raw(k.clone(), v.clone());
        }

        if let Some(body) = body {
            body_json = match serde_json::to_string(&body) {
                Ok(j) => j,
                Err(e) => return Err(format!("Error json-encoding body: {}", e)),
            };
            req.set_body(body_json)
        }

        let work = client.request(req).and_then(|res| {
            let status = res.status();
            res.body().collect()
                .and_then(move |chunk| {
                    let mut buffer: Vec<u8> = Vec::new();
                    for i in chunk {
                        buffer.append(&mut i.to_vec());
                    }

                    if buffer.len() == 0 {
                        buffer = b"{}".to_vec();
                    }
                    Ok(buffer)
                })
                .map(move |buffer| {
                    if !status.is_success() {
                        return Err(format!("Failed request to {}: HTTP {}\n---\n{}\n---", path, status,
                                   String::from_utf8_lossy(&buffer)));
                    }

                    return Ok(InternalResp { data: buffer });
                })
        });

        match core.run(work) {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("Error sending request: {:?}", e)),
        }
    }
}
