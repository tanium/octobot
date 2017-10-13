use http_client::HTTPClient;
use github::models::*;

pub trait Session : Send + Sync {
    fn user(&self) -> &User;
    fn github_host(&self) -> &str;
    fn github_token(&self) -> &str;
    fn get_pull_request(&self, owner: &str, repo: &str, number: u32)
                        -> Result<PullRequest, String>;
    fn get_pull_requests(&self, owner: &str, repo: &str, state: Option<&str>, head: Option<&str>)
                         -> Result<Vec<PullRequest>, String>;

    fn create_pull_request(&self, owner: &str, repo: &str, title: &str, body: &str, head: &str,
                           base: &str)
                           -> Result<PullRequest, String>;

    fn get_pull_request_labels(&self, owner: &str, repo: &str, number: u32)
                               -> Result<Vec<Label>, String>;

    fn get_pull_request_commits(&self, owner: &str, repo: &str, number: u32)
                                    -> Result<Vec<Commit>, String>;

    fn assign_pull_request(&self, owner: &str, repo: &str, number: u32, assignees: Vec<String>)
                           -> Result<AssignResponse, String>;

    fn comment_pull_request(&self, owner: &str, repo: &str, number: u32, comment: &str)
                            -> Result<(), String>;
    fn create_branch(&self, owner: &str, repo: &str, branch_name: &str, sha: &str) -> Result<(), String>;
    fn delete_branch(&self, owner: &str, repo: &str, branch_name: &str) -> Result<(), String>;
    fn get_statuses(&self, owner: &str, repo: &str, ref_name: &str) -> Result<Vec<Status>, String>;
    fn create_status(&self, owner: &str, repo: &str, ref_name: &str, status: &Status) -> Result<(), String>;
}

pub struct GithubSession {
    client: HTTPClient,
    host: String,
    token: String,
    user: User,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct EmptyResponse {
    empty: Option<String>,
}

impl GithubSession {
    pub fn new(host: &str, token: &str) -> Result<GithubSession, String> {
        let api_base = if host == "github.com" {
            "https://api.github.com".to_string()
        } else {
            format!("https://{}/api/v3", host)
        };

        let client = HTTPClient::new(&api_base)
            .with_headers(hashmap!{
                "Accept" => "application/vnd.github.black-cat-preview+json, application/vnd.github.v3+json".to_string(),
                "Content-Type" => "application/json".to_string(),
                "Authorization" => format!("Token {}", token),
            });

        // make sure we can auth as this user befor handing out session.
        let user: User = match client.get("/user") {
            Ok(u) => u,
            Err(e) => return Err(format!("Error authenticating with token: {}", e)),
        };


        Ok(GithubSession {
            client: client,
            user: user,
            host: host.to_string(),
            token: token.to_string(),
       })
    }
}

impl Session for GithubSession {
    fn user(&self) -> &User {
        return &self.user;
    }

    fn github_host(&self) -> &str {
        &self.host
    }

    fn github_token(&self) -> &str {
        &self.token
    }

    fn get_pull_request(&self, owner: &str, repo: &str, number: u32)
                            -> Result<PullRequest, String> {
        self.client.get(&format!("repos/{}/{}/pulls/{}", owner, repo, number))
    }

    fn get_pull_requests(&self, owner: &str, repo: &str, state: Option<&str>,
                             head: Option<&str>)
                             -> Result<Vec<PullRequest>, String> {
        let prs: Vec<PullRequest> = try!(self.client
            .get(&format!("repos/{}/{}/pulls?state={}&head={}",
                          owner,
                          repo,
                          state.unwrap_or(""),
                          head.unwrap_or(""))));

        let prs: Vec<PullRequest> = prs.into_iter()
            .filter(|p| {
                if let Some(head) = head {
                    p.head.ref_name == head || p.head.sha == head
                } else {
                    true
                }
            })
            .collect();
        Ok(prs)
    }

    fn create_pull_request(&self, owner: &str, repo: &str, title: &str, body: &str,
                               head: &str, base: &str)
                               -> Result<PullRequest, String> {
        #[derive(Serialize)]
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

        self.client.post(&format!("repos/{}/{}/pulls", owner, repo), &pr)
    }

    fn get_pull_request_labels(&self, owner: &str, repo: &str, number: u32)
                                   -> Result<Vec<Label>, String> {
        self.client.get(&format!("repos/{}/{}/issues/{}/labels", owner, repo, number))
    }

    fn get_pull_request_commits(&self, owner: &str, repo: &str, number: u32)
                                    -> Result<Vec<Commit>, String> {
        self.client.get(&format!("repos/{}/{}/pulls/{}/commits", owner, repo, number))
    }

    fn assign_pull_request(&self, owner: &str, repo: &str, number: u32,
                               assignees: Vec<String>)
                               -> Result<AssignResponse, String> {
        #[derive(Serialize)]
        struct AssignPR {
            assignees: Vec<String>,
        }

        let body = AssignPR { assignees: assignees };

        self.client.post(&format!("repos/{}/{}/issues/{}/assignees", owner, repo, number),
                         &body)
    }

    fn comment_pull_request(&self, owner: &str, repo: &str, number: u32, comment: &str)
                                -> Result<(), String> {
        #[derive(Serialize)]
        struct CommentPR {
            body: String,
        }
        let body = CommentPR { body: comment.to_string() };

        let _: EmptyResponse = try!(self.client.post(&format!("repos/{}/{}/issues/{}/comments", owner, repo, number), &body));
        Ok(())
    }

    fn create_branch(&self, owner: &str, repo: &str, branch_name: &str, sha: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct CreateRef {
            #[serde(rename = "ref")]
            ref_name: String,
            sha: String,
        }

        let body = CreateRef { ref_name: format!("refs/heads/{}", branch_name), sha: sha.into() };

        let _: EmptyResponse = try!(self.client.post(&format!("repos/{}/{}/git/refs", owner, repo), &body));
        Ok(())
    }

    fn delete_branch(&self, owner: &str, repo: &str, branch_name: &str) -> Result<(), String> {
        let _: EmptyResponse = try!(self.client.delete(&format!("repos/{}/{}/git/refs/heads/{}", owner, repo, branch_name)));
        Ok(())
    }

    fn get_statuses(&self, owner: &str, repo: &str, ref_name: &str) -> Result<Vec<Status>, String> {
        self.client.get(&format!("repos/{}/{}/commits/{}/statuses", owner, repo, ref_name))
    }

    fn create_status(&self, owner: &str, repo: &str, ref_name: &str, status: &Status) -> Result<(), String> {
        let _: Status = try!(
            self.client.post(&format!("repos/{}/{}/commits/{}/statuses", owner, repo, ref_name), status));
        Ok(())
    }
}


