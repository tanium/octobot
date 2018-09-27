use futures::future::{self, Future};
use futures::Stream;
use hyper;
use hyper::header::USER_AGENT;
use hyper::{Body, Request};
use hyper_rustls::HttpsConnector;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use serde_json;
use std::collections::HashMap;

use errors;
use errors::*;

pub use hyper::Method;

pub struct HTTPClient {
    api_base: String,
    headers: HashMap<&'static str, String>,
}

struct InternalResp {
    data: hyper::Chunk,
    headers: hyper::HeaderMap,
    status: hyper::StatusCode,
}

pub struct HTTPResponse<T> {
    pub item: T,
    pub headers: hyper::HeaderMap,
    pub status: hyper::StatusCode,
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

    pub fn get<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de::<T, ()>(Method::GET, path, None).map(|res| res.item)
    }

    pub fn get_async<T>(&self, path: &str) -> impl Future<Item = HTTPResponse<T>, Error = errors::Error> + Send
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de_async::<T, ()>(Method::GET, path, None)
    }

    pub fn post<T, U: Serialize>(&self, path: &str, body: &U) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de::<T, U>(Method::POST, path, Some(body))
            .map(|res| res.item)
    }

    pub fn post_async<T, U: Serialize>(
        &self,
        path: &str,
        body: &U,
    ) -> impl Future<Item = HTTPResponse<T>, Error = errors::Error> + Send
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de_async::<T, U>(Method::POST, path, Some(body))
    }

    pub fn post_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        self.request_void::<U>(Method::POST, path, Some(body)).map(|_| ())
    }

    pub fn post_void_async<U: Serialize>(
        &self,
        path: &str,
        body: &U,
    ) -> impl Future<Item = HTTPResponse<()>, Error = errors::Error> + Send {
        self.request_void_async::<U>(Method::POST, path, Some(body))
    }

    pub fn put<T, U: Serialize>(&self, path: &str, body: &U) -> Result<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de::<T, U>(Method::PUT, path, Some(body))
            .map(|res| res.item)
    }

    pub fn put_async<T, U: Serialize>(
        &self,
        path: &str,
        body: &U,
    ) -> impl Future<Item = HTTPResponse<T>, Error = errors::Error> + Send
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de_async::<T, U>(Method::PUT, path, Some(body))
    }

    pub fn put_void<U: Serialize>(&self, path: &str, body: &U) -> Result<()> {
        self.request_void::<U>(Method::PUT, path, Some(body)).map(|_| ())
    }

    pub fn put_void_async<U: Serialize>(
        &self,
        path: &str,
        body: &U,
    ) -> impl Future<Item = HTTPResponse<()>, Error = errors::Error> + Send {
        self.request_void_async::<U>(Method::PUT, path, Some(body))
    }

    pub fn delete_void(&self, path: &str) -> Result<()> {
        self.request_void::<()>(Method::DELETE, path, None).map(|_| ())
    }

    pub fn delete_void_async(&self, path: &str) -> impl Future<Item = HTTPResponse<()>, Error = errors::Error> + Send {
        self.request_void_async::<()>(Method::DELETE, path, None)
    }

    pub fn request_de<T, U: Serialize>(&self, method: Method, path: &str, body: Option<&U>) -> Result<HTTPResponse<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.request_de_async(method, path, body)
            .wait()
            .map_err(|e| Error::from(format!("Error waiting for HTTP response: {}", e)))
    }

    pub fn request_raw<U: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&U>,
    ) -> Result<HTTPResponse<Vec<u8>>> {
        self.request_raw_async(method, path, body)
            .wait()
            .map_err(|e| Error::from(format!("Error waiting for HTTP response: {}", e)))
    }

    pub fn request_void<U: Serialize>(&self, method: Method, path: &str, body: Option<&U>) -> Result<HTTPResponse<()>> {
        self.request_void_async(method, path, body)
            .wait()
            .map_err(|e| Error::from(format!("Error waiting for HTTP response: {}", e)))
    }

    pub fn request_de_async<T, U: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&U>,
    ) -> impl Future<Item = HTTPResponse<T>, Error = errors::Error> + Send
    where
        T: DeserializeOwned + Send + 'static,
    {
        let path = path.to_string();
        self.request_async(method, &path, body)
            .or_else(|_| Err("HTTP Request was cancelled".into()))
            .and_then(move |res| {
                if res.status.is_redirection() {
                    warn!(
                        "Received redirection when expected to receive data to deserialize. \
                         Request: {}; Headers: {:?}",
                        path, res.headers
                    );
                }

                serde_json::from_slice::<T>(&res.data)
                    .map_err(|e| {
                        format!(
                            "Error parsing response: {}\n---\n{}\n---",
                            e,
                            String::from_utf8_lossy(&res.data)
                        ).into()
                    }).map(|obj| HTTPResponse {
                        item: obj,
                        headers: res.headers,
                        status: res.status,
                    })
            })
    }

    pub fn request_void_async<U: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&U>,
    ) -> impl Future<Item = HTTPResponse<()>, Error = errors::Error> + Send {
        self.request_async(method, path, body)
            .or_else(|_| Err("HTTP Request was cancelled".into()))
            .map(|res| HTTPResponse {
                item: (),
                headers: res.headers,
                status: res.status,
            })
    }

    pub fn request_raw_async<U: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&U>,
    ) -> impl Future<Item = HTTPResponse<Vec<u8>>, Error = errors::Error> + Send {
        self.request_async(method, path, body)
            .or_else(|_| Err("HTTP Request was cancelled".into()))
            .map(|res| HTTPResponse {
                item: res.data.to_vec(),
                headers: res.headers,
                status: res.status,
            })
    }

    fn request_async<U: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&U>,
    ) -> Box<Future<Item = InternalResp, Error = errors::Error> + Send> {
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
                return Box::new(future::err(format!("Error parsing url: {}: {}", url, e).into()));
            }
        };

        let headers = self.headers.clone();

        let mut req = Request::builder();
        req.method(method).uri(url).header(USER_AGENT, "octobot");

        for (k, v) in &headers {
            match v.parse::<hyper::header::HeaderValue>() {
                Ok(v) => {
                    req.header(k.clone(), v);
                }
                Err(e) => {
                    error!("Skipping invalid header: {} => {}: {}", k, v, e);
                }
            }
        }

        let req = match body {
            Some(body) => match serde_json::to_string(&body) {
                Ok(j) => req.body(Body::from(j)),
                Err(e) => {
                    return Box::new(future::err(format!("Error json-encoding body: {}", e).into()));
                }
            },
            None => req.body(Body::empty()),
        };
        let req = match req {
            Ok(r) => r,
            Err(e) => {
                return Box::new(future::err(format!("Error building HTTP request: {}", e).into()));
            }
        };

        let path = path.to_string();

        // TODO: I wonder if these objects are expensive to create and we should be sharing them across requests?
        let client = hyper::Client::builder().build(HttpsConnector::new(4));

        Box::new(
            client
                .request(req)
                .and_then(|res| {
                    let status = res.status();
                    let headers = res.headers().clone();
                    res.into_body().concat2().map(move |buffer| (status, headers, buffer))
                }).map_err(|e| {
                    let msg = format!("Error in HTTP request: {}", e);
                    error!("{}", msg);
                    msg.into()
                }).and_then(move |(status, headers, buffer)| {
                    debug!(
                        "Response: HTTP {}\n---\n{}\n---",
                        status,
                        String::from_utf8_lossy(&buffer)
                    );
                    if !status.is_success() && !status.is_redirection() {
                        future::err(
                            format!(
                                "Failed request to {}: HTTP {}\n---\n{}\n---",
                                path,
                                status,
                                String::from_utf8_lossy(&buffer)
                            ).into(),
                        )
                    } else {
                        future::ok(InternalResp {
                            data: buffer,
                            headers: headers,
                            status: status,
                        })
                    }
                }),
        )
    }
}
