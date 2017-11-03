use futures::Stream;
use futures::future::Future;
use futures::sync::oneshot;
use hyper;
use hyper::{Method, Request};
use hyper::header::UserAgent;
use hyper_rustls::HttpsConnector;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use serde_json;
use std::collections::HashMap;
use tokio_core::reactor::Remote;

pub struct HTTPClient {
    api_base: String,
    headers: HashMap<&'static str, String>,
    core_remote: Remote,
}

struct InternalResp {
    data: hyper::Chunk,
}

type InternalResponseResult = Result<InternalResp, String>;
type FutureInternalResponse = oneshot::Receiver<InternalResponseResult>;

impl HTTPClient {
    pub fn new(core_remote: Remote, api_base: &str) -> HTTPClient {
        HTTPClient {
            api_base: api_base.into(),
            headers: HashMap::new(),
            core_remote: core_remote,
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

    fn request_de<T: DeserializeOwned, U: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&U>,
    ) -> Result<T, String> {
        let res = self.request_sync(method, path, body)?;

        let obj: T = match serde_json::from_slice(&res.data) {
            Ok(obj) => obj,
            Err(e) => {
                return Err(format!("Could not parse response: {}\n---\n{}\n---", e, String::from_utf8_lossy(&res.data)))
            }
        };

        Ok(obj)
    }

    fn request_void<U: Serialize>(&self, method: Method, path: &str, body: Option<&U>) -> Result<(), String> {
        self.request_sync(method, path, body)?;
        Ok(())
    }

    fn request_sync<U: Serialize>(&self, method: Method, path: &str, body: Option<&U>) -> Result<InternalResp, String> {
        match self.request_async(method, path, body).wait() {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(e),
            Err(e) => return Err(format!("{}", e)),
        }
    }

    fn request_async<U: Serialize>(&self, method: Method, path: &str, body: Option<&U>) -> FutureInternalResponse {
        let (tx, rx) = oneshot::channel::<InternalResponseResult>();

        let send_future = |it| if let Err(_) = tx.send(it) {
            error!("Error sending on future channel");
        };

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
        let url: hyper::Uri = match url.parse() {
            Ok(u) => u,
            Err(e) => {
                let _ = send_future(Err(format!("Error: {}", e)));
                return rx;
            }
        };

        let headers = self.headers.clone();

        let mut req = Request::new(method, url);
        req.headers_mut().set(UserAgent::new("octobot"));

        for (k, v) in &headers {
            req.headers_mut().set_raw(k.clone(), v.clone());
        }

        if let Some(body) = body {
            let body_json = match serde_json::to_string(&body) {
                Ok(j) => j,
                Err(e) => {
                    send_future(Err(format!("Error json-encoding body: {}", e)));
                    return rx;
                }
            };
            req.set_body(body_json)
        }

        let path = path.to_string();

        self.core_remote.spawn(move |handle| {
            // TODO: I wonder if these objects are expensive to create and we should be sharing them across requests?
            let https = HttpsConnector::new(4, &handle);
            let client = hyper::Client::configure().connector(https).build(&handle);

            client
                .request(req)
                .map_err(|e| {
                    error!("Error in HTTP request: {}", e);
                })
                .and_then(|res| {
                    let status = res.status();
                    res.body()
                        .concat2()
                        .map_err(|e| {
                            error!("Error in HTTP request: {}", e);
                        })
                        .map(move |buffer| {
                            debug!("Response: HTTP {}\n---\n{}\n---", status, String::from_utf8_lossy(&buffer));
                            if !status.is_success() {
                                send_future(Err(format!(
                                    "Failed request to {}: HTTP {}\n---\n{}\n---",
                                    path,
                                    status,
                                    String::from_utf8_lossy(&buffer)
                                )));
                            } else {
                                send_future(Ok(InternalResp { data: buffer }));
                            }
                        })
                })
        });

        rx
    }
}
