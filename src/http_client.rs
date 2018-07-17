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

use errors;
use errors::*;

pub struct HTTPClient {
    api_base: String,
    headers: HashMap<&'static str, String>,
    core_remote: Remote,
}

struct InternalResp {
    data: hyper::Chunk,
    headers: hyper::Headers,
    status: hyper::StatusCode,
}

pub struct HTTPResponse<T> {
    pub item: T,
    pub headers: hyper::Headers,
    pub status: hyper::StatusCode,
}

type InternalResponseResult = Result<InternalResp>;
type FutureInternalResponse = oneshot::Receiver<InternalResponseResult>;

pub type FutureResult<T> = Box<Future<Item = T, Error = errors::Error> + Send + 'static>;

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

    pub fn get<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de::<T, ()>(Method::Get, path, None).map(|res| res.item)
    }

    pub fn get_async<T>(&self, path: &str) -> FutureResult<HTTPResponse<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de_async::<T, ()>(Method::Get, path, None)
    }

    pub fn post<T, U: Serialize>(&self, path: &str, body: &U) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de::<T, U>(Method::Post, path, Some(body)).map(|res| res.item)
    }

    pub fn post_async<T, U: Serialize>(&self, path: &str, body: &U) -> FutureResult<HTTPResponse<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de_async::<T, U>(Method::Post, path, Some(body))
    }

    pub fn post_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        self.request_void::<U>(Method::Post, path, Some(body)).map(|_| ())
    }

    pub fn post_void_async<U: Serialize>(&self, path: &str, body: &U) -> FutureResult<HTTPResponse<()>> {
        self.request_void_async::<U>(Method::Post, path, Some(body))
    }

    pub fn put<T, U: Serialize>(&self, path: &str, body: &U) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de::<T, U>(Method::Put, path, Some(body)).map(|res| res.item)
    }

    pub fn put_async<T, U: Serialize>(&self, path: &str, body: &U) -> FutureResult<HTTPResponse<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de_async::<T, U>(Method::Put, path, Some(body))
    }

    pub fn put_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        self.request_void::<U>(Method::Put, path, Some(body)).map(|_| ())
    }

    pub fn put_void_async<U: Serialize>(&self, path: &str, body: &U) -> FutureResult<HTTPResponse<()>> {
        self.request_void_async::<U>(Method::Put, path, Some(body))
    }

    pub fn delete_void(&self, path: &str) -> Result<()> {
        self.request_void::<()>(Method::Delete, path, None).map(|_| ())
    }

    pub fn delete_void_async(&self, path: &str) -> FutureResult<HTTPResponse<()>> {
        self.request_void_async::<()>(Method::Delete, path, None)
    }

    // `spawn` is necesary for driving futures returned by async methods and any combinations
    // applied on top of them w/o needing to `wait` on the result
    pub fn spawn<F>(&self, fut: F)
    where
        F: Future<Item = (), Error = ()> + Send + 'static,
    {
        self.core_remote.spawn(move |_| fut);
    }

    pub fn request_de<T, U: Serialize>(&self, method: Method, path: &str, body: Option<&U>) -> Result<HTTPResponse<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de_async(method, path, body).wait().map_err(|e| {
            Error::from(format!("Error waiting for HTTP response: {}", e))
        })
    }

    pub fn request_void<U: Serialize>(&self, method: Method, path: &str, body: Option<&U>) -> Result<HTTPResponse<()>> {
        self.request_void_async(method, path, body).wait().map_err(|e| {
            Error::from(format!("Error waiting for HTTP response: {}", e))
        })
    }

    pub fn request_de_async<T, U: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&U>,
    ) -> FutureResult<HTTPResponse<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let path = path.to_string();
        Box::new(
            self.request_async(method, &path, body)
                .or_else(|_| Err("HTTP Request was cancelled".into()))
                .and_then(move |res| {
                    res.and_then(|res| {
                        if res.status.is_redirection() {
                            warn!(
                                "Received redirection when expected to receive data to deserialize. \
                                 Request: {}; Headers: {}",
                                path,
                                res.headers
                            );
                        }

                        serde_json::from_slice::<T>(&res.data)
                            .map_err(|e| {
                                format!(
                                    "Error parsing response: {}\n---\n{}\n---",
                                    e,
                                    String::from_utf8_lossy(&res.data)
                                ).into()
                            })
                            .map(|obj| {
                                HTTPResponse {
                                    item: obj,
                                    headers: res.headers,
                                    status: res.status,
                                }
                            })
                    })
                }),
        )
    }

    pub fn request_void_async<U: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&U>,
    ) -> FutureResult<HTTPResponse<()>> {
        Box::new(
            self.request_async(method, path, body)
                .or_else(|_| Err("HTTP Request was cancelled".into()))
                .and_then(|res| {
                    res.map(|r| {
                        HTTPResponse {
                            item: (),
                            headers: r.headers,
                            status: r.status,
                        }
                    })
                }),
        )
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
                send_future(Err(format!("Error parsing url: {}: {}", url, e).into()));
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
                    send_future(Err(format!("Error json-encoding body: {}", e).into()));
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
                    let headers = res.headers().clone();
                    res.body()
                        .concat2()
                        .map_err(|e| {
                            error!("Error in HTTP request: {}", e);
                        })
                        .map(move |buffer| {
                            debug!("Response: HTTP {}\n---\n{}\n---", status, String::from_utf8_lossy(&buffer));
                            if !status.is_success() && !status.is_redirection() {
                                send_future(Err(
                                    format!(
                                        "Failed request to {}: HTTP {}\n---\n{}\n---",
                                        path,
                                        status,
                                        String::from_utf8_lossy(&buffer)
                                    ).into(),
                                ));
                            } else {
                                send_future(Ok(InternalResp {
                                    data: buffer,
                                    headers: headers,
                                    status: status,
                                }));
                            }
                        })
                })
        });

        rx
    }
}
