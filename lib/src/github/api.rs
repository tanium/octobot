use async_trait::async_trait;
use failure::format_err;
use log::{error, info};
use serde_derive::{Deserialize, Serialize};
use std::sync::Arc;

use crate::errors::*;
use crate::github::models::*;
use crate::github::models_checks::*;
use crate::http_client::HTTPClient;
use crate::jwt;
use crate::metrics::Metrics;

#[async_trait]
pub trait Session: Send + Sync {
    fn bot_name(&self) -> &str;
    fn github_host(&self) -> &str;
    fn github_token(&self) -> &str;
    fn github_app_id(&self) -> Option<u32>;

    async fn get_pull_request(&self, owner: &str, repo: &str, number: u32) -> Result<PullRequest>;
    async fn get_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
        head: Option<&str>,
    ) -> Result<Vec<PullRequest>>;
    async fn get_pull_requests_by_commit(
        &self,
        owner: &str,
        repo: &str,
        commit: &str,
        head: Option<&str>,
    ) -> Result<Vec<PullRequest>>;

    async fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<PullRequest>;

    async fn get_pull_request_labels(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
    ) -> Result<Vec<Label>>;

    async fn add_pull_request_labels(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        labels: Vec<String>,
    ) -> Result<()>;

    async fn get_pull_request_commits(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
    ) -> Result<Vec<Commit>>;

    async fn get_pull_request_reviews(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
    ) -> Result<Vec<Review>>;

    async fn assign_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        assignees: Vec<String>,
    ) -> Result<()>;

    async fn request_review(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        reviewers: Vec<String>,
    ) -> Result<()>;

    async fn comment_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        comment: &str,
    ) -> Result<()>;
    async fn create_branch(
        &self,
        owner: &str,
        repo: &str,
        branch_name: &str,
        sha: &str,
    ) -> Result<()>;
    async fn delete_branch(&self, owner: &str, repo: &str, branch_name: &str) -> Result<()>;
    async fn approve_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        commit_hash: &str,
        comment: Option<&str>,
    ) -> Result<()>;
    async fn get_timeline(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
    ) -> Result<Vec<TimelineEvent>>;

    // checks api
    async fn get_suites(&self, pr: &PullRequest) -> Result<Vec<CheckSuite>>;
    async fn get_check_run(&self, pr: &PullRequest, id: u32) -> Result<CheckRun>;
    async fn create_check_run(&self, pr: &PullRequest, run: &CheckRun) -> Result<u32>;
    async fn update_check_run(
        &self,
        pr: &PullRequest,
        check_run_id: u32,
        run: &CheckRun,
    ) -> Result<()>;
    async fn get_team_members(&self, org: &str, team: &str) -> Result<Vec<User>>;
}

#[async_trait]
pub trait GithubSessionFactory: Send + Sync {
    async fn new_session(&self, owner: &str, repo: &str) -> Result<GithubSession>;
    async fn get_token_org(&self, org: &str) -> Result<String>;
    async fn get_token_repo(&self, owner: &str, repo: &str) -> Result<String>;
    fn bot_name(&self) -> String;
}

pub fn api_base(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com".to_string()
    } else {
        format!("https://{}/api/v3", host)
    }
}

pub struct GithubApp {
    host: String,
    app_id: u32,
    // DER formatted API private key
    app_key: Vec<u8>,
    app: Option<App>,
    metrics: Option<Arc<Metrics>>,
}

pub struct GithubOauthApp {
    host: String,
    api_token: String,
    user: Option<User>,
    metrics: Option<Arc<Metrics>>,
}

impl GithubApp {
    pub async fn new(
        host: &str,
        app_id: u32,
        app_key: &[u8],
        metrics: Option<Arc<Metrics>>,
    ) -> Result<GithubApp> {
        let mut github = GithubApp {
            host: host.into(),
            app_id,
            app_key: app_key.into(),
            app: None,
            metrics,
        };

        github.app = Some(
            github
                .new_client()?
                .get("/app")
                .await
                .map_err(|e| format_err!("Error authenticating to github with token: {}", e))?,
        );

        info!("Logged in as GithubApp {}", github.bot_name());

        Ok(github)
    }

    fn new_client(&self) -> Result<HTTPClient> {
        let jwt_token = jwt::new_token(self.app_id, &self.app_key);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::ACCEPT,
            "application/vnd.github.v3+json".parse().unwrap(),
        );
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", jwt_token).parse().unwrap(),
        );

        let client = HTTPClient::new_with_headers(&api_base(&self.host), headers)?;
        if let Some(ref m) = self.metrics {
            Ok(client.with_metrics(
                m.github_api_responses.clone(),
                m.github_api_duration.clone(),
            ))
        } else {
            Ok(client)
        }
    }

    async fn new_token(&self, installation_url: &str) -> Result<String> {
        let client = self.new_client()?;

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
        let installation: Installation = client.get(installation_url).await?;
        // Get a new access token for this id
        let token: AccessToken = client
            .post(
                &format!("/app/installations/{}/access_tokens", installation.id),
                &String::new(),
            )
            .await?;
        Ok(token.token)
    }
}

#[async_trait]
impl GithubSessionFactory for GithubApp {
    fn bot_name(&self) -> String {
        format!(
            "{}[bot]",
            self.app.clone().map(|a| a.name).unwrap_or_default()
        )
    }

    async fn get_token_org(&self, org: &str) -> Result<String> {
        self.new_token(&format!("/orgs/{}/installation", org)).await
    }

    async fn get_token_repo(&self, owner: &str, repo: &str) -> Result<String> {
        self.new_token(&format!("/repos/{}/{}/installation", owner, repo))
            .await
    }

    async fn new_session(&self, owner: &str, repo: &str) -> Result<GithubSession> {
        GithubSession::new(
            &self.host,
            &self.bot_name(),
            &self.get_token_repo(owner, repo).await?,
            Some(self.app_id),
            self.metrics.clone(),
        )
    }
}

impl GithubOauthApp {
    pub async fn new(
        host: &str,
        api_token: &str,
        metrics: Option<Arc<Metrics>>,
    ) -> Result<GithubOauthApp> {
        let mut github = GithubOauthApp {
            host: host.into(),
            api_token: api_token.into(),
            user: None,
            metrics,
        };

        github.user = Some(
            github
                .new_session("", "")
                .await?
                .client
                .get::<User>("/user")
                .await
                .map_err(|e| format_err!("Error authenticating to github with token: {}", e))?,
        );

        info!("Logged in as OAuth app {}", github.bot_name());

        Ok(github)
    }
}

#[async_trait]
impl GithubSessionFactory for GithubOauthApp {
    fn bot_name(&self) -> String {
        self.user
            .clone()
            .map(|a| a.login().into())
            .unwrap_or_default()
    }

    async fn get_token_org(&self, _org: &str) -> Result<String> {
        Ok(self.api_token.clone())
    }

    async fn get_token_repo(&self, _owner: &str, _repo: &str) -> Result<String> {
        Ok(self.api_token.clone())
    }

    async fn new_session(&self, _owner: &str, _repo: &str) -> Result<GithubSession> {
        GithubSession::new(
            &self.host,
            &self.bot_name(),
            &self.api_token,
            None,
            self.metrics.clone(),
        )
    }
}

pub struct GithubSession {
    client: HTTPClient,
    host: String,
    token: String,
    bot_name: String,
    app_id: Option<u32>,
}

impl GithubSession {
    pub fn new(
        host: &str,
        bot_name: &str,
        token: &str,
        app_id: Option<u32>,
        metrics: Option<Arc<Metrics>>,
    ) -> Result<GithubSession> {
        let mut headers = reqwest::header::HeaderMap::new();

        let accept_headers = vec![
            // standard header
            "application/vnd.github.v3+json",
            // timeline api
            "application/vnd.github.mockingbird-preview",
            // draft PRs
            "application/vnd.github.shadow-cat-preview+json",
            // checks API
            "application/vnd.github.antiope-preview+json",
        ]
        .join(",");

        headers.append(
            reqwest::header::ACCEPT,
            accept_headers.as_str().parse().unwrap(),
        );
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Token {}", token).parse().unwrap(),
        );

        let client = HTTPClient::new_with_headers(&api_base(host), headers)?;
        let client = match metrics {
            None => client,
            Some(ref m) => client.with_metrics(
                m.github_api_responses.clone(),
                m.github_api_duration.clone(),
            ),
        };

        Ok(GithubSession {
            client,
            bot_name: bot_name.to_string(),
            host: host.to_string(),
            token: token.to_string(),
            app_id,
        })
    }

    async fn do_get_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        head: Option<&str>,
        paging_url: &str,
        err_fmt: &str,
    ) -> Result<Vec<PullRequest>> {
        let mut pull_requests = vec![];
        let mut page = 1;
        loop {
            let mut next_prs: Vec<PullRequest> = match self
                .client
                .get::<Vec<PullRequest>>(&format!("{}&page={}", paging_url, page))
                .await
                .map_err(|e| format_err!("{}: {}", err_fmt, e))
                .map(|prs| {
                    prs.into_iter()
                        .filter(|p| {
                            if let Some(head) = head {
                                p.head.ref_name == head || p.head.sha == head
                            } else {
                                true
                            }
                        })
                        .collect::<Vec<_>>()
                }) {
                Ok(r) => r,
                Err(e) => return Err(e),
            };

            if next_prs.is_empty() {
                break;
            }

            for pull_request in &mut next_prs {
                if pull_request.reviews.is_none() {
                    match self
                        .get_pull_request_reviews(owner, repo, pull_request.number)
                        .await
                    {
                        Ok(r) => pull_request.reviews = Some(r),
                        Err(e) => error!("Error refetching pull request reviews: {}", e),
                    };
                }
            }

            pull_requests.extend(next_prs.into_iter());
            page += 1;
        }
        Ok(pull_requests)
    }
}

#[async_trait]
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

    fn github_app_id(&self) -> Option<u32> {
        self.app_id
    }

    async fn get_pull_request(&self, owner: &str, repo: &str, number: u32) -> Result<PullRequest> {
        let pull_request: Result<PullRequest> = self
            .client
            .get(&format!("repos/{}/{}/pulls/{}", owner, repo, number))
            .await
            .map_err(|e| format_err!("Error looking up PR: {}/{} #{}: {}", owner, repo, number, e));
        let mut pull_request = pull_request?;

        // Always fetch PR's reviewers. Users get removed from requested_reviewers after they submit their review. :cry:
        if pull_request.reviews.is_none() {
            match self.get_pull_request_reviews(owner, repo, number).await {
                Ok(r) => pull_request.reviews = Some(r),
                Err(e) => error!("Error refetching pull request reviews: {}", e),
            };
        }

        Ok(pull_request)
    }

    async fn get_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
        head: Option<&str>,
    ) -> Result<Vec<PullRequest>> {
        let paging_url = &format!(
            "repos/{}/{}/pulls?state={}&head={}&per_page=100",
            owner,
            repo,
            state.unwrap_or(""),
            head.unwrap_or(""),
        );
        let err_fmt = &format!("Error looking up PRs for commit: {}/{}", owner, repo);
        return self
            .do_get_pull_requests(owner, repo, head, paging_url, err_fmt)
            .await;
    }

    async fn get_pull_requests_by_commit(
        &self,
        owner: &str,
        repo: &str,
        commit: &str,
        head: Option<&str>,
    ) -> Result<Vec<PullRequest>> {
        let paging_url = &format!(
            "repos/{}/{}/commits/{}/pulls?head={}&per_page=100",
            owner,
            repo,
            commit,
            head.unwrap_or(""),
        );
        let err_fmt = &format!(
            "Error looking up PRs for commit: {}/{}/{}",
            owner, repo, commit
        );
        return self
            .do_get_pull_requests(owner, repo, head, paging_url, err_fmt)
            .await;
    }

    async fn create_pull_request(
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

        self.client
            .post(&format!("repos/{}/{}/pulls", owner, repo), &pr)
            .await
    }

    async fn get_pull_request_labels(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
    ) -> Result<Vec<Label>> {
        self.client
            .get(&format!(
                "repos/{}/{}/issues/{}/labels",
                owner, repo, number
            ))
            .await
            .map_err(|e| {
                format_err!(
                    "error looking up pr labels: {}/{} #{}: {}",
                    owner,
                    repo,
                    number,
                    e
                )
            })
    }

    async fn add_pull_request_labels(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        labels: Vec<String>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct AddLabels {
            labels: Vec<String>,
        }

        let body = AddLabels { labels };

        self.client
            .post_void(
                &format!("repos/{}/{}/issues/{}/labels", owner, repo, number),
                &body,
            )
            .await
            .map_err(|e| format_err!("Error adding label: {}/{} #{}: {}", owner, repo, number, e))
    }

    async fn get_pull_request_commits(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
    ) -> Result<Vec<Commit>> {
        let mut result = vec![];
        let mut page = 1;

        loop {
            let next: Vec<Commit> = self
                .client
                .get(&format!(
                    "repos/{}/{}/pulls/{}/commits?per_page=100&page={}",
                    owner, repo, number, page,
                ))
                .await
                .map_err(|e| {
                    format_err!(
                        "Error looking up PR commits: {}/{} #{}: {}",
                        owner,
                        repo,
                        number,
                        e
                    )
                })?;

            if next.is_empty() {
                break;
            }

            result.extend(next.into_iter());
            page += 1;
        }

        Ok(result)
    }

    async fn get_pull_request_reviews(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
    ) -> Result<Vec<Review>> {
        self.client
            .get(&format!(
                "repos/{}/{}/pulls/{}/reviews",
                owner, repo, number
            ))
            .await
            .map_err(|e| {
                format_err!(
                    "Error looking up PR reviews: {}/{} #{}: {}",
                    owner,
                    repo,
                    number,
                    e
                )
            })
    }

    async fn assign_pull_request(
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

        let body = AssignPR { assignees };

        self.client
            .post_void(
                &format!("repos/{}/{}/issues/{}/assignees", owner, repo, number),
                &body,
            )
            .await
            .map_err(|e| format_err!("Error assigning PR: {}/{} #{}: {}", owner, repo, number, e))
    }

    async fn request_review(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        reviewers: Vec<String>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct ReviewPR {
            reviewers: Vec<String>,
        }

        let body = ReviewPR { reviewers };

        self.client
            .post_void(
                &format!(
                    "repos/{}/{}/pulls/{}/requested_reviewers",
                    owner, repo, number
                ),
                &body,
            )
            .await
            .map_err(|e| {
                format_err!(
                    "Error requesting review for PR: {}/{} #{}: {}",
                    owner,
                    repo,
                    number,
                    e
                )
            })
    }

    async fn comment_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        comment: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct CommentPR {
            body: String,
        }
        let body = CommentPR {
            body: comment.to_string(),
        };

        self.client
            .post_void(
                &format!("repos/{}/{}/issues/{}/comments", owner, repo, number),
                &body,
            )
            .await
            .map_err(|e| {
                format_err!(
                    "Error commenting on PR: {}/{} #{}: {}",
                    owner,
                    repo,
                    number,
                    e
                )
            })
    }

    async fn create_branch(
        &self,
        owner: &str,
        repo: &str,
        branch_name: &str,
        sha: &str,
    ) -> Result<()> {
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

        self.client
            .post_void(&format!("repos/{}/{}/git/refs", owner, repo), &body)
            .await
            .map_err(|e| {
                format_err!(
                    "Error creating branch {}/{} {}, {}: {}",
                    owner,
                    repo,
                    branch_name,
                    sha,
                    e
                )
            })
    }

    async fn delete_branch(&self, owner: &str, repo: &str, branch_name: &str) -> Result<()> {
        self.client
            .delete_void(&format!(
                "repos/{}/{}/git/refs/heads/{}",
                owner, repo, branch_name
            ))
            .await
            .map_err(|e| {
                format_err!(
                    "Error deleting branch {}/{} {}: {}",
                    owner,
                    repo,
                    branch_name,
                    e
                )
            })
    }

    async fn approve_pull_request(
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
            .post_void(
                &format!("repos/{}/{}/pulls/{}/reviews", owner, repo, number),
                &body,
            )
            .await
            .map_err(|e| format_err!("Error approving PR {}/{} #{}: {}", owner, repo, number, e))
    }

    async fn get_timeline(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
    ) -> Result<Vec<TimelineEvent>> {
        let mut events = vec![];
        let mut page = 1;
        loop {
            let url = format!(
                "repos/{}/{}/issues/{}/timeline?per_page=100&page={}",
                owner, repo, number, page
            );
            let next_events: Vec<TimelineEvent> = match self.client.get(&url).await.map_err(|e| {
                format_err!(
                    "Error getting timeline for PR: {}/{} #{}: {}",
                    owner,
                    repo,
                    number,
                    e
                )
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

    async fn get_suites(&self, pr: &PullRequest) -> Result<Vec<CheckSuite>> {
        #[derive(Deserialize)]
        pub struct CheckSuiteList {
            pub total_count: u32,
            pub check_suites: Vec<CheckSuite>,
        }

        let app_id = match self.app_id {
            Some(id) => id,
            None => {
                return Err(format_err!("get_suites only supported for GitHub Apps"));
            }
        };

        self.client
            .get::<CheckSuiteList>(&format!(
                "/repos/{}/commits/{}/check-suites?app_id={}",
                pr.base.repo.full_name, pr.head.sha, app_id
            ))
            .await
            .map(|list| list.check_suites)
            .map_err(|e| {
                format_err!(
                    "Error getting suites for {} {}: {}",
                    pr.base.repo.full_name,
                    pr.head.sha,
                    e
                )
            })
    }

    async fn get_check_run(&self, pr: &PullRequest, id: u32) -> Result<CheckRun> {
        self.client
            .get(&format!(
                "/repos/{}/check-runs/{}",
                pr.base.repo.full_name, id
            ))
            .await
            .map_err(|e| {
                format_err!(
                    "Error getting check run #{} for {}: {}",
                    id,
                    pr.base.repo.full_name,
                    e
                )
            })
    }

    async fn create_check_run(&self, pr: &PullRequest, run: &CheckRun) -> Result<u32> {
        #[derive(Deserialize, Serialize, Clone, Debug)]
        pub struct Resp {
            pub id: u32,
        }

        self.client
            .post::<Resp, CheckRun>(
                &format!("/repos/{}/check-runs", pr.base.repo.full_name),
                run,
            )
            .await
            .map_err(|e| {
                format_err!(
                    "Error creating check run for {}: {}",
                    pr.base.repo.full_name,
                    e
                )
            })
            .map(|r| r.id)
    }

    async fn update_check_run(
        &self,
        pr: &PullRequest,
        check_run_id: u32,
        run: &CheckRun,
    ) -> Result<()> {
        self.client
            .client
            .patch(&format!(
                "{}/repos/{}/check-runs/{}",
                self.client.api_base, pr.base.repo.full_name, check_run_id
            ))
            .json(&run)
            .send()
            .await
            .map_err(|e| {
                format_err!(
                    "Error updating check run #{} for {}: {}\n\nPayload: {:#?}",
                    check_run_id,
                    pr.base.repo.full_name,
                    e,
                    run
                )
            })?
            .error_for_status()?;
        Ok(())
    }

    async fn get_team_members(&self, org: &str, team: &str) -> Result<Vec<User>> {
        let users: Result<Vec<User>> = self
            .client
            .get(&format!("orgs/{}/teams/{}", org, team))
            .await
            .map_err(|e| format_err!("Error looking up team members {}/{}: {}", org, team, e));

        Ok(users?)
    }
}
