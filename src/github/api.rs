use tokio_core::reactor::Remote;

use errors::*;
use github::models::*;
use http_client::HTTPClient;

pub trait Session: Send + Sync {
    fn user(&self) -> &User;
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
    ) -> Result<()>;

    fn request_review(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        reviewers: Vec<String>,
    ) -> Result<()>;

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

pub struct GithubSession {
    client: HTTPClient,
    host: String,
    token: String,
    user: User,
}

impl GithubSession {
    pub fn new(core_remote: Remote, host: &str, token: &str) -> Result<GithubSession> {
        let api_base = if host == "github.com" {
            "https://api.github.com".to_string()
        } else {
            format!("https://{}/api/v3", host)
        };

        let client = HTTPClient::new(core_remote, &api_base).with_headers(hashmap!{
                // Standard accept header is "application/vnd.github.v3+json".
                // The "mockingbird-preview" allows us to use the timeline api.
                // cf. https://developer.github.com/enterprise/2.13/v3/issues/timeline/
                "Accept" => "application/vnd.github.mockingbird-preview".to_string(),
                "Content-Type" => "application/json".to_string(),
                "Authorization" => format!("Token {}", token),
            });

        // make sure we can auth as this user befor handing out session.
        let user: User = client.get("/user").map_err(|e| {
            Error::from(format!("Error authenticating to github with token: {}", e))
        })?;

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
    ) -> Result<()> {
        #[derive(Serialize)]
        struct AssignPR {
            assignees: Vec<String>,
        }

        let body = AssignPR { assignees: assignees };

        self.client
            .post_void(&format!("repos/{}/{}/issues/{}/assignees", owner, repo, number), &body)
            .map_err(|e| format!("Error assigning PR: {}/{} #{}: {}", owner, repo, number, e).into())
    }

    fn request_review(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        reviewers: Vec<String>,
    ) -> Result<()>
    {
        #[derive(Serialize)]
        struct ReviewPR {
            reviewers: Vec<String>,
        }

        let body = ReviewPR{ reviewers: reviewers };

        self.client
            .post_void(&format!("repos/{}/{}/issues/{}/requested_reviewers", owner, repo, number), &body)
            .map_err(|e| format!("Error requesting review for PR: {}/{} #{}: {}", owner, repo, number, e).into())
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
