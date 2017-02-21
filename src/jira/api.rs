use std::io::Read;
use hyper;
use hyper::header::{Accept, Authorization, Basic, ContentType, qitem, UserAgent};
use hyper::method::Method;
use hyper::mime::{Mime, TopLevel, SubLevel};
use serde_json;
use serde::ser::Serialize;
use serde::de::Deserialize;

use jira::models::*;

pub trait Session : Send + Sync {
    fn get_transitions(&self, key: &str) -> Result<Vec<Transition>, String>;

    fn transition_issue(&self, key: &str, transition: &TransitionRequest) -> Result<(), String>;

    fn comment_issue(&self, key: &str, comment: &str) -> Result<(), String>;
}

pub struct JiraSession {
    client: JiraClient,
}

#[derive(Deserialize)]
struct AuthResp {
    pub name: String,
}

impl JiraSession {
    pub fn new(host: &str, user: &str, pass: &str) -> Result<JiraSession, String> {
        let jira_base;
        if host.starts_with("http") {
            jira_base = host.into();
        } else {
            jira_base = format!("https://{}", host);
        }

        let api_base = format!("{}/rest/api/2", jira_base);

        let client = JiraClient {
            api_base: api_base,
            jira_base: jira_base.clone(),
            user: user.to_string(),
            pass: pass.to_string(),
        };

        match client.get::<AuthResp>(&format!("{}/rest/auth/1/session", jira_base)) {
            Ok(a) => info!("Logged into JIRA as {}", a.name),
            Err(e) => return Err(format!("Error authenticating to JIRA: {}", e)),
        };

        Ok(JiraSession{
            client: client,
        })
    }
}

impl Session for JiraSession {
    fn get_transitions(&self, key: &str) -> Result<Vec<Transition>, String> {
        #[derive(Deserialize)]
        struct TransitionsResp {
            transitions: Vec<Transition>,
        }
        let resp: TransitionsResp = try!(self.client.get(&format!("/issue/{}/transitions?expand=transitions.fields", key)));
        Ok(resp.transitions)
    }

    fn transition_issue(&self, key: &str, req: &TransitionRequest) -> Result<(), String> {
        // TODO: would be nice to specialize for () return type...
        #[derive(Deserialize)]
        struct Resp {
            pub fix_json_parse: Option<String>,
        }
        try!(self.client.post::<Resp, TransitionRequest>(&format!("/issue/{}/transitions", key), &req));
        Ok(())
    }

    fn comment_issue(&self, key: &str, comment: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct CommentReq {
            body: String,
        }
        let req = CommentReq { body: comment.to_string() };
        try!(self.client.post::<Comment, CommentReq>(&format!("/issue/{}/comment", key), &req));
        Ok(())
    }
}

pub struct JiraClient {
    api_base: String,
    jira_base: String,
    user: String,
    pass: String,
}

// TODO: lots of duplication with GithubClient...
impl JiraClient {
    pub fn get<T: Deserialize>(&self, path: &str) -> Result<T, String> {
        self.request::<T, String>(Method::Get, path, None)
    }

    pub fn post<T: Deserialize, E: Serialize>(&self, path: &str, body: &E) -> Result<T, String> {
        self.request::<T, E>(Method::Post, path, Some(body))
    }

    fn request<T: Deserialize, E: Serialize>(&self, method: Method, path: &str, body: Option<&E>)
                                             -> Result<T, String> {
        let url;
        if path.starts_with(&self.jira_base) {
            url = path.into();
        } else if path.starts_with("/") {
            url = self.api_base.clone() + path;
        } else {
            url = self.api_base.clone() + "/" + path;
        }

        let body_json: String;

        let client = hyper::client::Client::new();
        let mut req = client.request(method, url.as_str())
            .header(UserAgent("octobot".to_string()))
            .header(Accept(vec![qitem(Mime(TopLevel::Application, SubLevel::Json, vec![]))]))
            .header(ContentType(Mime(TopLevel::Application, SubLevel::Json, vec![])))
            .header(Authorization(Basic {
                username: self.user.clone(),
                password: Some(self.pass.clone()),
            }));

        if let Some(body) = body {
            body_json = match serde_json::to_string(&body) {
                Ok(j) => j,
                Err(e) => return Err(format!("Error json-encoding body: {}", e)),
            };
            req = req.body(&body_json)
        }

        let res = req.send();

        match res {
            Ok(mut res) => {
                let mut res_str = String::new();
                res.read_to_string(&mut res_str).unwrap_or(0);
                if res.status.is_success() {
                    if res_str.len() == 0 {
                        res_str = "{}".into();
                    }
                    let obj: T = match serde_json::from_str(&res_str) {
                        Ok(obj) => obj,
                        Err(e) => return Err(format!("Coudl not parse response: {}", e)),
                    };
                    Ok(obj)
                } else {
                    Err(format!("HTTP {} -- {}", res.status, res_str))
                }
            }
            Err(e) => Err(format!("Error sending request to JIRA: {}", e)),
        }
    }
}
