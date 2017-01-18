use std::io::Read;
use super::super::hyper;
use super::super::hyper::header::{Accept, Authorization, Bearer, ContentType, qitem};
use super::super::hyper::method::Method;
use super::super::hyper::mime::{Mime, TopLevel, SubLevel};
use super::super::rustc_serialize::{json, Decodable, Encodable};

use super::models::*;

pub struct Session {
    client: GithubClient,
    user: User,
}

impl Session {
    pub fn new(host: &str, token: &str) -> Result<Session, String> {
        let api_base = if host.to_string() == "github.com" {
            "https://api.github.com".to_string()
        } else {
            format!("https://{}/api/v3", host)
        };

        let client = GithubClient {
            host: host.to_string(),
            token: token.to_string(),
            api_base: api_base,
        };

        // make sure we can auth as this user befor handing out session.
        let user: User = match client.get("/user") {
            Ok(u) => u,
            Err(e) => return Err(format!("Error authenticating with token: {}", e)),
        };

        Ok(Session {
            client: client,
            user: user,
        })
    }

    pub fn user(&self) -> &User {
        return &self.user;
    }

    pub fn get_pull_request(&self,
                            owner: &str,
                            repo: &str,
                            number: u32)
                            -> Result<PullRequest, String> {
        self.client.get(format!("repos/{}/{}/pulls/{}", owner, repo, number).as_str())
    }

    pub fn get_pull_requests(&self,
                             owner: &str,
                             repo: &str,
                             state: Option<&str>,
                             head: Option<&str>)
                             -> Result<Vec<PullRequest>, String> {
        self.client.get(format!("repos/{}/{}/pulls?state={}&head={}",
                                owner,
                                repo,
                                state.unwrap_or(""),
                                head.unwrap_or(""))
            .as_str())
    }

    pub fn create_pull_request(&self,
                               owner: &str,
                               repo: &str,
                               title: &str,
                               body: &str,
                               head: &str,
                               base: &str)
                               -> Result<PullRequest, String> {
        #[derive(RustcEncodable)]
        struct CreatePR {
            title: String,
            body: String,
            head: String,
            base: String,
        }
        let pr = CreatePR {
            title: title.to_string(),
            body: body.to_string(),
            head: head.to_string(),
            base: base.to_string(),
        };

        self.client.post(format!("repos/{}/{}/pulls", owner, repo).as_str(), &pr)
    }

    pub fn get_pull_request_labels(&self,
                                   owner: &str,
                                   repo: &str,
                                   number: u32)
                                   -> Result<Vec<Label>, String> {

        self.client.get(format!("repos/{}/{}/issues/{}/labels", owner, repo, number).as_str())
    }

    pub fn assign_pull_request(&self,
                               owner: &str,
                               repo: &str,
                               number: u32,
                               assignees: Vec<String>)
                               -> Result<AssignResponse, String> {
        #[derive(RustcEncodable)]
        struct AssignPR {
            assignees: Vec<String>,
        }

        let body = AssignPR { assignees: assignees };

        self.client.post(format!("repos/{}/{}/issues/{}/assignees", owner, repo, number).as_str(),
                         &body)
    }

    pub fn create_merge_pull_request(&self,
                                     owner: &str,
                                     repo: &str,
                                     number: u32,
                                     target_branch: &str)
                                     -> Result<PullRequest, String> {

        Err("Not implemented".to_string())
    }
}

pub struct GithubClient {
    host: String,
    api_base: String,
    token: String,
}

impl GithubClient {
    pub fn get<T: Decodable>(&self, path: &str) -> Result<T, String> {
        self.request::<T, String>(Method::Get, path, None)
    }

    pub fn post<T: Decodable, E: Encodable>(&self, path: &str, body: &E) -> Result<T, String> {
        self.request::<T, E>(Method::Post, path, Some(body))
    }

    fn request<T: Decodable, E: Encodable>(&self,
                                           method: Method,
                                           path: &str,
                                           body: Option<&E>)
                                           -> Result<T, String> {
        let url;
        if path.starts_with("/") {
            url = self.api_base.clone() + path;
        } else {
            url = self.api_base.clone() + "/" + path;
        }

        let body_json: String;

        let client = hyper::client::Client::new();
        let mut req = client.request(method, url.as_str())
            .header(Accept(vec![qitem(Mime(TopLevel::Application, SubLevel::Json, vec![]))]))
            .header(ContentType(Mime(TopLevel::Application, SubLevel::Json, vec![])))
            .header(Authorization(Bearer { token: self.token.clone() }));

        if let Some(body) = body {
            body_json = match json::encode(body) {
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
                    let obj: T = match json::decode(&res_str) {
                        Ok(obj) => obj,
                        Err(e) => return Err(format!("Coudl not parse response: {}", e)),
                    };
                    Ok(obj)
                } else {
                    Err(format!("HTTP {} -- {}", res.status, res_str))
                }
            }
            Err(e) => Err(format!("Error sending to request github: {}", e)),
        }
    }
}
