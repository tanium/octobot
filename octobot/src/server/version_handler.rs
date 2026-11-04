use hyper::body::Incoming;
use hyper::{Request, Response};
use serde_derive::Serialize;
use ts_rs::TS;

use crate::http_util::{self, Body};
use crate::server::http::Handler;
use crate::version::VERSION;
use octobot_lib::errors::Result;

// Null when the binary was built without a version file.
#[derive(Serialize, TS)]
#[ts(export)]
struct VersionResp {
    commit: Option<String>,
}

pub struct VersionHandler;

impl VersionHandler {
    pub fn new() -> Box<VersionHandler> {
        Box::new(VersionHandler)
    }
}

#[async_trait::async_trait]
impl Handler for VersionHandler {
    async fn handle(&self, _: Request<Incoming>) -> Result<Response<Body>> {
        let resp = VersionResp {
            commit: VERSION.clone(),
        };
        Ok(http_util::new_json_resp(serde_json::to_string(&resp)?))
    }
}
