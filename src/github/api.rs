use tokio_core::reactor::Remote;

use errors::*;
use github::models::*;
use http_client::HTTPClient;
use jwt;

pub trait Session: Send + Sync {
    fn bot_name(&self) -> &str;
    fn github_host(&self) -> &str;
    fn github_token(&self) -> &str;
    fn get_pull_request(&self, owner: &str, repo: &str, number: u32) -> Result<PullRequest>;
    fn get_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
        head: Option<&str>,
    ) -> Result<Vec<PullRequest>>;

    fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<PullRequest>;

    fn get_pull_request_labels(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<Label>>;

    fn get_pull_request_commits(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<Commit>>;

    fn get_pull_request_reviews(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<Review>>;

    fn assign_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        assignees: Vec<String>,
    ) -> Result<AssignResponse>;

    fn comment_pull_request(&self, owner: &str, repo: &str, number: u32, comment: &str) -> Result<()>;
    fn create_branch(&self, owner: &str, repo: &str, branch_name: &str, sha: &str) -> Result<()>;
    fn delete_branch(&self, owner: &str, repo: &str, branch_name: &str) -> Result<()>;
    fn get_statuses(&self, owner: &str, repo: &str, ref_name: &str) -> Result<Vec<Status>>;
    fn create_status(&self, owner: &str, repo: &str, ref_name: &str, status: &Status) -> Result<()>;
    fn approve_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        commit_hash: &str,
        comment: Option<&str>,
    ) -> Result<()>;
    fn get_timeline(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<TimelineEvent>>;
}

pub fn api_base(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com".to_string()
    } else {
        format!("https://{}/api/v3", host)
    }
}

pub struct GithubApp {
    core_remote: Remote,
    host: String,
    app_id: u32,
    // DER formatted API private key
    app_key: Vec<u8>,
    app: Option<App>,
}

impl GithubApp {
    pub fn new(core_remote: Remote, host: &str, app_id: u32, app_key: &[u8]) -> Result<GithubApp> {
        let mut github = GithubApp {
            core_remote: core_remote,
            host: host.into(),
            app_id: app_id,
            app_key: app_key.into(),
            app: None,
        };

        github.app = Some(github.new_client().get("/app").map_err(|e| {
            Error::from(format!("Error authenticating to github with token: {}", e))
        })?);

        info!("Logged in as application {}", github.app_name());

        Ok(github)
    }

    fn app_name(&self) -> String {
        self.app.clone().map(|a| a.name).unwrap_or(String::new())
    }

    fn new_client(&self) -> HTTPClient {
        let jwt_token = jwt::new_token(self.app_id, &self.app_key);
        HTTPClient::new(self.core_remote.clone(), &api_base(&self.host)).with_headers(hashmap!{
            "Accept" => "application/vnd.github.machine-man-preview+json".to_string(),
            "Authorization" => format!("Bearer {}", jwt_token),
        })
    }

    pub fn new_token_org(&self, org: &str) -> Result<String> {
        let client = self.new_client();

        // All we care about for now is the installation id
        #[derive(Deserialize)]
        struct Installation {
            id: u32,
        }
        #[derive(Deserialize)]
        struct AccessToken {
            token: String,
        }

        // Lookup the installation id for this org/repo
        let installation: Installation = client.get(&format!("/orgs/{}/installation", org))?;
        // Get a new access token for this id
        let token: AccessToken =
            client.post(&format!("/installations/{}/access_tokens", installation.id), &String::new())?;
        Ok(token.token)
    }

    pub fn new_token(&self, owner: &str, repo: &str) -> Result<String> {
        let client = self.new_client();

        // All we care about for now is the installation id
        #[derive(Deserialize)]
        struct Installation {
            id: u32,
        }
        #[derive(Deserialize)]
        struct AccessToken {
            token: String,
        }

        // Lookup the installation id for this org/repo
        let installation: Installation = client.get(&format!("/repos/{}/{}/installation", owner, repo))?;
        // Get a new access token for this id
        let token: AccessToken =
            client.post(&format!("/installations/{}/access_tokens", installation.id), &String::new())?;
        Ok(token.token)
    }

    pub fn new_session(&self, owner: &str, repo: &str) -> Result<GithubSession> {
        GithubSession::new(self.core_remote.clone(), &self.host, &self.app_name(), &self.new_token(owner, repo)?)
    }
}

pub struct GithubSession {
    client: HTTPClient,
    host: String,
    token: String,
    bot_name: String,
}

impl GithubSession {
    pub fn new(core_remote: Remote, host: &str, app_name: &str, token: &str) -> Result<GithubSession> {
        let client = HTTPClient::new(core_remote, &api_base(host)).with_headers(hashmap!{
                // Standard accept header is "application/vnd.github.v3+json".
                // The "mockingbird-preview" allows us to use the timeline api.
                // cf. https://developer.github.com/enterprise/2.13/v3/issues/timeline/
                "Accept" => "application/vnd.github.mockingbird-preview".to_string(),
                "Content-Type" => "application/json".to_string(),
                "Authorization" => format!("Token {}", token),
            });

        Ok(GithubSession {
            client: client,
            bot_name: app_name.to_string() + "[bot]",
            host: host.to_string(),
            token: token.to_string(),
        })
    }
}

impl Session for GithubSession {
    fn bot_name(&self) -> &str {
        &self.bot_name
    }

    fn github_host(&self) -> &str {
        &self.host
    }

    fn github_token(&self) -> &str {
        &self.token
    }

    fn get_pull_request(&self, owner: &str, repo: &str, number: u32) -> Result<PullRequest> {
        self.client.get(&format!("repos/{}/{}/pulls/{}", owner, repo, number)).map_err(|e| {
            format!("Error looking up PR: {}/{} #{}: {}", owner, repo, number, e).into()
        })
    }

    fn get_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
        head: Option<&str>,
    ) -> Result<Vec<PullRequest>> {
        self.client
            .get::<Vec<PullRequest>>(
                &format!("repos/{}/{}/pulls?state={}&head={}", owner, repo, state.unwrap_or(""), head.unwrap_or("")),
            )
            .map_err(|e| format!("Error looking up PRs: {}/{}: {}", owner, repo, e).into())
            .map(|prs| {
                prs.into_iter()
                    .filter(|p| if let Some(head) = head {
                        p.head.ref_name == head || p.head.sha == head
                    } else {
                        true
                    })
                    .collect::<Vec<_>>()
            })
    }

    fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<PullRequest> {
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

    fn get_pull_request_labels(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<Label>> {
        self.client.get(&format!("repos/{}/{}/issues/{}/labels", owner, repo, number)).map_err(
            |e| {
                format!("error looking up pr labels: {}/{} #{}: {}", owner, repo, number, e).into()
            },
        )
    }

    fn get_pull_request_commits(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<Commit>> {
        self.client.get(&format!("repos/{}/{}/pulls/{}/commits", owner, repo, number)).map_err(
            |e| {
                format!("Error looking up PR commits: {}/{} #{}: {}", owner, repo, number, e).into()
            },
        )
    }

    fn get_pull_request_reviews(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<Review>> {
        self.client.get(&format!("repos/{}/{}/pulls/{}/reviews", owner, repo, number)).map_err(
            |e| {
                format!("Error looking up PR reviews: {}/{} #{}: {}", owner, repo, number, e).into()
            },
        )
    }

    fn assign_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        assignees: Vec<String>,
    ) -> Result<AssignResponse> {
        #[derive(Serialize)]
        struct AssignPR {
            assignees: Vec<String>,
        }

        let body = AssignPR { assignees: assignees };

        self.client
            .post(&format!("repos/{}/{}/issues/{}/assignees", owner, repo, number), &body)
            .map_err(|e| format!("Error assigning PR: {}/{} #{}: {}", owner, repo, number, e).into())
    }

    fn comment_pull_request(&self, owner: &str, repo: &str, number: u32, comment: &str) -> Result<()> {
        #[derive(Serialize)]
        struct CommentPR {
            body: String,
        }
        let body = CommentPR { body: comment.to_string() };

        self.client
            .post_void(&format!("repos/{}/{}/issues/{}/comments", owner, repo, number), &body)
            .map_err(|e| format!("Error commenting on PR: {}/{} #{}: {}", owner, repo, number, e).into())
    }

    fn create_branch(&self, owner: &str, repo: &str, branch_name: &str, sha: &str) -> Result<()> {
        #[derive(Serialize)]
        struct CreateRef {
            #[serde(rename = "ref")]
            ref_name: String,
            sha: String,
        }

        let body = CreateRef {
            ref_name: format!("refs/heads/{}", branch_name),
            sha: sha.into(),
        };

        self.client.post_void(&format!("repos/{}/{}/git/refs", owner, repo), &body).map_err(|e| {
            format!("Error creating branch {}/{} {}, {}: {}", owner, repo, branch_name, sha, e).into()
        })
    }

    fn delete_branch(&self, owner: &str, repo: &str, branch_name: &str) -> Result<()> {
        self.client
            .delete_void(&format!("repos/{}/{}/git/refs/heads/{}", owner, repo, branch_name))
            .map_err(|e| format!("Error deleting branch {}/{} {}: {}", owner, repo, branch_name, e).into())
    }

    fn get_statuses(&self, owner: &str, repo: &str, ref_name: &str) -> Result<Vec<Status>> {
        self.client
            .get(&format!("repos/{}/{}/commits/{}/statuses", owner, repo, ref_name))
            .map_err(|e| format!("Error getting statuses {}/{} {}: {}", owner, repo, ref_name, e).into())
    }

    fn create_status(&self, owner: &str, repo: &str, ref_name: &str, status: &Status) -> Result<()> {
        self.client
            .post_void(&format!("repos/{}/{}/commits/{}/statuses", owner, repo, ref_name), status)
            .map_err(|e| format!("Error creating status {}/{} {}: {}", owner, repo, ref_name, e).into())
    }

    fn approve_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        commit_hash: &str,
        comment: Option<&str>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct ReviewReq {
            body: String,
            event: String,
            commit_id: String,
        }

        let body = ReviewReq {
            body: comment.unwrap_or("").into(),
            event: "APPROVE".into(),
            // Require the commit hash here as well to make sure we avoid a race condition and are
            // approving the right commit.
            commit_id: commit_hash.into(),
        };

        self.client
            .post_void(&format!("repos/{}/{}/pulls/{}/reviews", owner, repo, number), &body)
            .map_err(|e| format!("Error approving PR {}/{} #{}: {}", owner, repo, number, e).into())
    }

    fn get_timeline(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<TimelineEvent>> {
        let mut events = vec![];
        let mut page = 1;
        loop {
            let url = format!("repos/{}/{}/issues/{}/timeline?per_page=100&page={}", owner, repo, number, page);
            let next_events: Vec<TimelineEvent> = match self.client.get(&url).map_err(|e| {
                format!("Error getting timeline for PR: {}/{} #{}: {}", owner, repo, number, e).into()
            }) {
                Ok(r) => r,
                Err(e) => return Err(e),
            };

            if next_events.is_empty() {
                break;
            }

            events.extend(next_events.into_iter());
            page += 1;
        }

        Ok(events)
    }
}
