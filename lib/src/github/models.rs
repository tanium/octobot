use anyhow::anyhow;
use serde_derive::{Deserialize, Serialize};
use url::Url;

use crate::errors::*;

pub fn is_main_branch(branch_name: &str) -> bool {
    branch_name == "master" || branch_name == "develop" || branch_name == "main"
}

// An incomplete container for all the kinds of events that we care about.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct HookBody {
    pub repository: Option<Repo>,
    pub sender: User,

    pub action: Option<String>,
    pub issue: Option<Issue>,
    pub comment: Option<Comment>,
    pub pull_request: Option<PullRequest>,
    pub review: Option<Review>,
    pub label: Option<Label>,
    pub requested_reviewer: Option<User>,

    // push event related stuff
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub compare: Option<String>,
    pub forced: Option<bool>,
    pub deleted: Option<bool>,
    pub created: Option<bool>,
    pub commits: Option<Vec<PushCommit>>,

    pub check_suite: Option<CheckHook>,
    pub check_run: Option<CheckHook>,
}

/// The parts of a check_suite or check_run webhook octobot reads. The full `CheckSuite` and
/// `CheckRun` models reject statuses webhooks send, such as "requested".
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct CheckHook {
    pub head_sha: String,
    /// Kept as a string: webhooks send conclusions `Conclusion` does not parse, such as "skipped".
    pub conclusion: Option<String>,
    /// Open pull requests whose head is the checked branch and commit. These may belong to
    /// another repository, such as the upstream of a fork. GitHub leaves out pull requests from
    /// forks.
    pub pull_requests: Vec<CheckPullRequest>,
}

impl CheckHook {
    /// Cancelled runs are left out: fail-fast cancels a failed run's siblings.
    pub fn failed(&self) -> bool {
        matches!(
            self.conclusion.as_deref(),
            Some("failure" | "timed_out" | "startup_failure")
        )
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct CheckPullRequest {
    pub number: u32,
    pub base: CheckPullRequestBase,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct CheckPullRequestBase {
    pub repo: CheckRepo,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct CheckRepo {
    pub id: u64,
}

impl HookBody {
    pub fn new() -> HookBody {
        HookBody {
            repository: Some(Repo::new()),
            sender: User::new(""),
            action: None,
            issue: None,
            comment: None,
            pull_request: None,
            review: None,
            label: None,
            requested_reviewer: None,
            ref_name: None,
            after: None,
            before: None,
            compare: None,
            forced: None,
            deleted: None,
            created: None,
            commits: None,
            check_suite: None,
            check_run: None,
        }
    }

    pub fn ref_name(&self) -> &str {
        self.ref_name.as_deref().unwrap_or_default()
    }

    pub fn after(&self) -> &str {
        self.after.as_deref().unwrap_or_default()
    }

    pub fn before(&self) -> &str {
        self.before.as_deref().unwrap_or_default()
    }

    pub fn forced(&self) -> bool {
        self.forced.unwrap_or(false)
    }

    pub fn created(&self) -> bool {
        self.created.unwrap_or(false)
    }

    pub fn deleted(&self) -> bool {
        self.deleted.unwrap_or(false)
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct App {
    pub id: u32,
    pub owner: User,
    pub name: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Team {
    pub id: u32,
    pub slug: String,
}

impl Team {
    pub fn new(id: u32, slug: &str) -> Team {
        Team {
            id,
            slug: slug.into(),
        }
    }
}

impl PartialEq for Team {
    fn eq(&self, other: &Self) -> bool {
        self.slug == other.slug
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct User {
    pub id: u32,
    pub login: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "type", default)]
    pub user_type: Option<String>,
}

impl User {
    pub fn new(login: &str) -> User {
        User {
            id: 0,
            login: Some(login.to_string()),
            name: Some(login.to_string()),
            user_type: None,
        }
    }

    pub fn login(&self) -> &str {
        self.login
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or_default()
    }

    pub fn is_bot(&self) -> bool {
        self.user_type.as_deref() == Some("Bot")
    }
}

impl PartialEq for User {
    fn eq(&self, other: &User) -> bool {
        self.login() == other.login()
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Repo {
    pub id: u64,
    pub html_url: String,
    pub full_name: String,
    pub name: String,
    pub owner: User,
    pub archived: Option<bool>,
}

impl Repo {
    pub fn new() -> Repo {
        Repo {
            id: 0,
            html_url: String::new(),
            full_name: String::new(),
            name: String::new(),
            owner: User::new(""),
            archived: Some(false),
        }
    }

    pub fn parse(html_url: &str) -> Result<Repo> {
        let url = Url::parse(html_url)?;
        let segments: Vec<&str> = match url.path_segments() {
            Some(s) => s.filter(|p| !p.is_empty()).collect(),
            None => return Err(anyhow!("No path segments in URL")),
        };
        if segments.len() != 2 {
            return Err(anyhow!("Expected only two path segments!"));
        }

        let user = segments[0];
        let repo = segments[1];

        Ok(Repo {
            id: 0,
            html_url: html_url.to_string(),
            full_name: format!("{}/{}", user, repo),
            name: repo.to_string(),
            owner: User::new(user),
            archived: Some(false),
        })
    }

    pub fn archived(&self) -> bool {
        self.archived.unwrap_or(false)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BranchRef {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
    pub user: User,
    pub repo: Repo,
}

impl BranchRef {
    pub fn new(name: &str) -> BranchRef {
        BranchRef {
            ref_name: name.into(),
            sha: String::new(),
            user: User::new(""),
            repo: Repo::new(),
        }
    }
}

pub trait PullRequestLike: Send + Sync {
    fn user(&self) -> &User;
    fn teams(&self) -> Vec<Team>;
    fn title(&self) -> &str;
    fn html_url(&self) -> &str;
    fn number(&self) -> u32;
    fn has_commits(&self) -> bool;
    fn repo(&self) -> Option<&Repo>;
}

// Everyone to notify about an issue or pull request. A bare `PullRequest` does not
// implement this: reviewers leave `requested_reviewers` once they review, so only
// a `ReviewedPullRequest` knows all of them.
pub trait Notifiable: PullRequestLike {
    fn assignees_and_reviewers(&self) -> Vec<User>;
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct PullRequest {
    pub title: String,
    pub body: Option<String>,
    pub number: u32,
    pub html_url: String,
    pub state: String,
    pub user: User,
    pub merged: Option<bool>,
    pub merge_commit_sha: Option<String>,
    pub assignees: Vec<User>,
    pub head: BranchRef,
    pub base: BranchRef,
    pub requested_reviewers: Option<Vec<User>>,
    pub requested_teams: Option<Vec<Team>>,
    pub draft: Option<bool>,
    pub labels: Option<Vec<Label>>,
    pub updated_at: Option<String>,
}

// https://docs.github.com/en/enterprise-cloud@latest/rest/pulls/pulls?apiVersion=2022-11-28#list-pull-requests-files
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct PullRequestFile {
    pub filename: String,
}

impl PullRequest {
    pub fn new() -> PullRequest {
        PullRequest {
            title: String::new(),
            body: None,
            number: 0,
            html_url: String::new(),
            state: "open".into(),
            user: User::new(""),
            merged: None,
            merge_commit_sha: None,
            assignees: vec![],
            requested_reviewers: None,
            requested_teams: None,
            head: BranchRef::new(""),
            base: BranchRef::new(""),
            draft: None,
            labels: None,
            updated_at: None,
        }
    }

    pub fn is_merged(&self) -> bool {
        self.merged.unwrap_or(false)
    }

    pub fn is_draft(&self) -> bool {
        self.draft.unwrap_or(false) || self.title.to_lowercase().starts_with("wip:")
    }
}

// A pull request with everyone who has reviewed it. Webhooks and pull request
// lookups never include those reviewers; they come only from a reviews lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewedPullRequest {
    pull_request: PullRequest,
    reviewers: Vec<User>,
}

impl ReviewedPullRequest {
    // `reviewers` are the authors of the PR's submitted reviews.
    pub fn new(pull_request: PullRequest, reviewers: Vec<User>) -> ReviewedPullRequest {
        ReviewedPullRequest {
            pull_request,
            reviewers,
        }
    }

    pub fn all_reviewers(&self) -> Vec<User> {
        let mut reviewers: Vec<User> = vec![];
        let requested = self.pull_request.requested_reviewers.iter().flatten();
        for user in requested.chain(self.reviewers.iter()) {
            if !reviewers.iter().any(|r| r.login() == user.login()) {
                reviewers.push(user.clone());
            }
        }
        reviewers
    }
}

impl std::ops::Deref for ReviewedPullRequest {
    type Target = PullRequest;

    fn deref(&self) -> &PullRequest {
        &self.pull_request
    }
}

impl PullRequestLike for &ReviewedPullRequest {
    fn user(&self) -> &User {
        &self.pull_request.user
    }

    fn teams(&self) -> Vec<Team> {
        (&self.pull_request).teams()
    }

    fn title(&self) -> &str {
        &self.pull_request.title
    }

    fn html_url(&self) -> &str {
        &self.pull_request.html_url
    }

    fn number(&self) -> u32 {
        self.pull_request.number
    }

    fn has_commits(&self) -> bool {
        true
    }

    fn repo(&self) -> Option<&Repo> {
        Some(&self.pull_request.base.repo)
    }
}

impl Notifiable for &ReviewedPullRequest {
    fn assignees_and_reviewers(&self) -> Vec<User> {
        let mut assignees = self.pull_request.assignees.clone();
        assignees.extend(self.all_reviewers());
        assignees
    }
}

impl PullRequestLike for &PullRequest {
    fn user(&self) -> &User {
        &self.user
    }

    fn teams(&self) -> Vec<Team> {
        let mut teams = vec![];
        if let Some(ref rt) = self.requested_teams {
            teams.extend(rt.iter().cloned());
        }
        teams
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn html_url(&self) -> &str {
        &self.html_url
    }

    fn number(&self) -> u32 {
        self.number
    }

    fn has_commits(&self) -> bool {
        true
    }

    fn repo(&self) -> Option<&Repo> {
        Some(&self.base.repo)
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Issue {
    pub number: u32,
    pub html_url: String,
    pub title: String,
    pub user: User,
    pub assignees: Vec<User>,
}

impl PullRequestLike for &Issue {
    fn user(&self) -> &User {
        &self.user
    }

    fn teams(&self) -> Vec<Team> {
        vec![]
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn html_url(&self) -> &str {
        &self.html_url
    }

    fn number(&self) -> u32 {
        self.number
    }

    fn has_commits(&self) -> bool {
        false
    }

    fn repo(&self) -> Option<&Repo> {
        None
    }
}

impl Notifiable for &Issue {
    fn assignees_and_reviewers(&self) -> Vec<User> {
        self.assignees.clone()
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Label {
    pub name: String,
}

impl Label {
    pub fn new(name: &str) -> Label {
        Label { name: name.into() }
    }
}

pub trait CommentLike: Send + Sync {
    fn user(&self) -> &User;
    fn body(&self) -> &str;
    fn html_url(&self) -> &str;
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Review {
    pub id: u64,
    pub state: String,
    pub body: Option<String>,
    pub html_url: String,
    pub user: User,
    // The commit reviewed.
    #[serde(default)]
    pub commit_id: Option<String>,
}

impl Review {
    pub fn new(body: &str, user: User) -> Review {
        Review {
            id: 0,
            state: "COMMENTED".into(),
            body: Some(body.into()),
            html_url: String::new(),
            user,
            commit_id: None,
        }
    }
}

impl CommentLike for &Review {
    fn body(&self) -> &str {
        self.body.as_deref().unwrap_or_default()
    }

    fn user(&self) -> &User {
        &self.user
    }

    fn html_url(&self) -> &str {
        &self.html_url
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: u64,
    #[serde(default)]
    pub pull_request_review_id: Option<u64>,
    pub commit_id: Option<String>,
    pub path: Option<String>,
    pub body: Option<String>,
    pub html_url: String,
    pub user: User,
}

impl CommentLike for &Comment {
    fn body(&self) -> &str {
        self.body.as_deref().unwrap_or_default()
    }

    fn user(&self) -> &User {
        &self.user
    }

    fn html_url(&self) -> &str {
        &self.html_url
    }
}

pub trait CommitLike: Send + Sync {
    fn sha(&self) -> &str;
    fn html_url(&self) -> &str;
    fn message(&self) -> &str;
}

// the 'Commit' objects that come from a push event have a different format from
// the api that lists commits for a PR
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct PushCommit {
    pub id: String,
    pub tree_id: String,
    pub message: String,
    pub url: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Commit {
    pub sha: String,
    pub html_url: String,
    pub commit: CommitDetails,
    pub author: Option<User>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct CommitDetails {
    pub message: String,
}

impl CommitLike for PushCommit {
    fn sha(&self) -> &str {
        &self.id
    }

    fn html_url(&self) -> &str {
        &self.url
    }

    fn message(&self) -> &str {
        &self.message
    }
}

impl CommitLike for &PushCommit {
    fn sha(&self) -> &str {
        &self.id
    }

    fn html_url(&self) -> &str {
        &self.url
    }

    fn message(&self) -> &str {
        &self.message
    }
}

impl CommitLike for Commit {
    fn sha(&self) -> &str {
        &self.sha
    }

    fn html_url(&self) -> &str {
        &self.html_url
    }

    fn message(&self) -> &str {
        &self.commit.message
    }
}

impl CommitLike for &Commit {
    fn sha(&self) -> &str {
        &self.sha
    }

    fn html_url(&self) -> &str {
        &self.html_url
    }

    fn message(&self) -> &str {
        &self.commit.message
    }
}

impl Commit {
    pub fn new() -> Commit {
        Commit {
            sha: String::new(),
            html_url: String::new(),
            author: None,
            commit: CommitDetails {
                message: String::new(),
            },
        }
    }

    pub fn short_hash(commit: &dyn CommitLike) -> &str {
        Commit::short_hash_str(commit.sha())
    }

    pub fn short_hash_str(hash: &str) -> &str {
        if hash.len() < 7 { hash } else { &hash[0..7] }
    }

    pub fn title(commit: &dyn CommitLike) -> String {
        commit.message().lines().next().unwrap_or("").into()
    }

    pub fn body(commit: &dyn CommitLike) -> String {
        let lines: Vec<&str> = commit
            .message()
            .lines()
            .skip(1)
            .skip_while(|l| l.trim().is_empty())
            .collect();
        lines.join("\n")
    }
}

impl PushCommit {
    pub fn new() -> PushCommit {
        PushCommit {
            id: String::new(),
            tree_id: String::new(),
            url: String::new(),
            message: String::new(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Status {
    pub state: String,
    pub target_url: Option<String>,
    pub context: Option<String>,
    pub description: Option<String>,
    pub creator: Option<User>,
    pub updated_at: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct TimelineEvent {
    pub id: Option<u32>,
    pub event: String,
    pub dismissed_review: Option<DismissedReview>,
    pub commit_id: Option<String>,
    pub user: Option<User>,
    pub html_url: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct DismissedReview {
    pub state: String,
    pub review_id: u32,
    pub dismissal_message: Option<String>,
    pub dismissal_commit_id: Option<String>,
}

impl TimelineEvent {
    pub fn new(event: &str) -> TimelineEvent {
        TimelineEvent {
            id: None,
            event: event.to_string(),
            dismissed_review: None,
            commit_id: None,
            user: None,
            html_url: None,
        }
    }

    pub fn new_dismissed_review(review: DismissedReview) -> TimelineEvent {
        let mut event = TimelineEvent::new("review_dismissed");
        event.dismissed_review = Some(review);
        event
    }

    pub fn new_review(commit_id: &str, review_id: u32, user: User, url: &str) -> TimelineEvent {
        let mut event = TimelineEvent::new("reviewed");
        event.id = Some(review_id);
        event.commit_id = Some(commit_id.into());
        event.user = Some(user);
        event.html_url = Some(url.into());
        event
    }

    pub fn is_review_dismissal(&self) -> bool {
        self.event == "review_dismissed"
    }

    pub fn is_review_dismissal_for(&self, commit_hash: &str) -> bool {
        if self.is_review_dismissal() {
            if let Some(ref review) = self.dismissed_review {
                if let Some(ref dismissal_commit_id) = review.dismissal_commit_id {
                    return review.state == "approved" && dismissal_commit_id == commit_hash;
                }
            }
        }

        false
    }

    pub fn dismissed_review_id(&self) -> Option<u32> {
        self.dismissed_review.as_ref().map(|r| r.review_id)
    }

    pub fn is_review(&self) -> bool {
        self.event == "reviewed"
    }

    pub fn is_review_for(&self, review_id: u32, commit_hash: &str) -> bool {
        if self.is_review() {
            if let Some(id) = self.id {
                if let Some(ref commit_id) = self.commit_id {
                    return id == review_id && commit_id == commit_hash;
                }
            }
        }

        false
    }

    pub fn review_user_message(&self, review_id: u32) -> String {
        if let Some(ref user) = self.user {
            if let Some(ref url) = self.html_url {
                format!("[{}]({})", user.login(), url)
            } else {
                format!("{} (review #{})", user.login(), review_id)
            }
        } else {
            format!("Unknown user (review #{})", review_id)
        }
    }
}

impl DismissedReview {
    pub fn by_commit(state: &str, commit_hash: &str, review_id: u32) -> DismissedReview {
        DismissedReview {
            review_id,
            state: state.into(),
            dismissal_commit_id: Some(commit_hash.into()),
            dismissal_message: None,
        }
    }

    pub fn by_user(state: &str, msg: &str) -> DismissedReview {
        DismissedReview {
            review_id: 0,
            state: state.into(),
            dismissal_commit_id: None,
            dismissal_message: Some(msg.into()),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct WebhookDelivery {
    pub id: u32,
    pub guid: String,
    pub redelivery: bool,
    pub status_code: u32,
    pub delivered_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_parse() {
        // note: trailing slash should'nt bother it
        let repo = Repo::parse("http://git.company.com/users/repo/").unwrap();

        assert_eq!("http://git.company.com/users/repo/", repo.html_url);
        assert_eq!("users/repo", repo.full_name);
        assert_eq!("users", repo.owner.login());
    }

    #[test]
    #[should_panic]
    fn test_repo_parse_no_repo() {
        Repo::parse("http://git.company.com/users/").unwrap();
    }

    #[test]
    #[should_panic]
    fn test_repo_parse_no_user() {
        Repo::parse("http://git.company.com/").unwrap();
    }

    #[test]
    #[should_panic]
    fn test_repo_parse_too_many_parts() {
        Repo::parse("http://git.company.com/users/repo/huh").unwrap();
    }

    #[test]
    fn test_hook_body_funcs() {
        // test defaults
        {
            let body = HookBody::new();
            assert_eq!("", body.ref_name());
            assert_eq!("", body.after());
            assert_eq!("", body.before());
            assert!(!body.forced());
            assert!(!body.created());
            assert!(!body.deleted());
        }

        // test values
        {
            let mut body = HookBody::new();
            body.ref_name = Some("the-ref".to_string());
            body.after = Some("after".to_string());
            body.before = Some("before".to_string());
            assert_eq!("the-ref", body.ref_name());
            assert_eq!("after", body.after());
            assert_eq!("before", body.before());
        }
        // test bools one by one
        {
            let mut body = HookBody::new();
            body.forced = Some(true);
            assert!(body.forced());
        }

        {
            let mut body = HookBody::new();
            body.created = Some(true);
            assert!(body.created());
        }
        {
            let mut body = HookBody::new();
            body.deleted = Some(true);
            assert!(body.deleted());
        }
    }

    #[test]
    fn test_commit_title() {
        let mut commit = Commit::new();

        commit.commit.message = "1 Hello there".into();
        assert_eq!("1 Hello there", Commit::title(&commit));
        assert_eq!("", Commit::body(&commit));

        commit.commit.message = "2 Hello there\n".into();
        assert_eq!("2 Hello there", Commit::title(&commit));
        assert_eq!("", Commit::body(&commit));

        commit.commit.message = "3 Hello there\n\n".into();
        assert_eq!("3 Hello there", Commit::title(&commit));
        assert_eq!("", Commit::body(&commit));

        commit.commit.message =
            "4 Hello there\n\nand then some more\nwith\nmultiple\n\nlines".into();
        assert_eq!("4 Hello there", Commit::title(&commit));
        assert_eq!(
            "and then some more\nwith\nmultiple\n\nlines",
            Commit::body(&commit)
        );

        commit.commit.message =
            "5 Hello there\r\n\r\nmaybe also support\r\ncarriage\r\nreturns?".into();
        assert_eq!("5 Hello there", Commit::title(&commit));
        assert_eq!(
            "maybe also support\ncarriage\nreturns?",
            Commit::body(&commit)
        );
    }

    #[test]
    fn test_commit_short_hash() {
        let mut commit = Commit::new();

        commit.sha = "".into();
        assert_eq!("", Commit::short_hash(&commit));

        commit.sha = "12345".into();
        assert_eq!("12345", Commit::short_hash(&commit));

        commit.sha = "123456".into();
        assert_eq!("123456", Commit::short_hash(&commit));

        commit.sha = "1234567".into();
        assert_eq!("1234567", Commit::short_hash(&commit));

        commit.sha = "12345678".into();
        assert_eq!("1234567", Commit::short_hash(&commit));
    }

    #[test]
    fn test_pr_assignees() {
        let mut pr = PullRequest::new();

        let users = vec![User::new("user1"), User::new("user2")];
        pr.assignees = users.clone();
        pr.requested_reviewers = None;

        let reviewed = ReviewedPullRequest::new(pr.clone(), vec![]);
        assert_eq!(users, (&reviewed).assignees_and_reviewers());

        let reviewers = vec![User::new("userC"), User::new("userD")];
        pr.requested_reviewers = Some(reviewers);

        let all_users = vec![
            User::new("user1"),
            User::new("user2"),
            User::new("userC"),
            User::new("userD"),
        ];
        let reviewed = ReviewedPullRequest::new(pr.clone(), vec![]);
        assert_eq!(all_users, (&reviewed).assignees_and_reviewers());

        let reviews = vec![
            Review::new("i like it", User::new("userE")),
            Review::new("i like it", User::new("userF")),
        ];

        let even_more_users = vec![
            User::new("user1"),
            User::new("user2"),
            User::new("userC"),
            User::new("userD"),
            User::new("userE"),
            User::new("userF"),
        ];
        let reviewed = ReviewedPullRequest::new(pr, reviews.into_iter().map(|r| r.user).collect());
        assert_eq!(even_more_users, (&reviewed).assignees_and_reviewers());
    }

    #[test]
    fn test_pr_all_reviewers_lists_each_reviewer_once() {
        let mut pr = PullRequest::new();
        pr.requested_reviewers = Some(vec![User::new("userC")]);

        // userE reviewed twice; userC reviewed and was requested again.
        let reviewers = vec![User::new("userE"), User::new("userC"), User::new("userE")];
        let reviewed = ReviewedPullRequest::new(pr, reviewers);

        assert_eq!(
            vec![User::new("userC"), User::new("userE")],
            reviewed.all_reviewers()
        );
    }

    #[test]
    fn test_pr_is_draft() {
        let mut pr = PullRequest::new();
        assert!(!pr.is_draft());

        pr.title = "WIP: doing some stuff".into();
        assert!(pr.is_draft());

        pr.title = "wip: doing some stuff".into();
        assert!(pr.is_draft());

        pr.title = "WIPWIPWIP: doing some stuff".into();
        assert!(!pr.is_draft());

        pr.title = "Wip: why would you even think about doing it this way?".into();
        assert!(pr.is_draft());

        pr.title = "Doing some stuff".into();
        assert!(!pr.is_draft());
        pr.draft = Some(false);
        assert!(!pr.is_draft());
        pr.draft = Some(true);
        assert!(pr.is_draft());
    }

    #[test]
    fn test_check_hook_parses_webhook_values() {
        let body: HookBody = serde_json::from_str(
            r#"{
                "sender": {"id": 1, "login": "github-actions[bot]"},
                "action": "requested",
                "check_suite": {
                    "head_sha": "ffff0000",
                    "status": "requested",
                    "conclusion": "skipped",
                    "pull_requests": [{
                        "number": 32,
                        "head": {"sha": "ffff0000"},
                        "base": {"ref": "main", "repo": {"id": 7, "name": "some-repo"}}
                    }]
                }
            }"#,
        )
        .unwrap();

        let suite = body.check_suite.unwrap();
        assert_eq!(Some("skipped"), suite.conclusion.as_deref());
        assert_eq!(
            vec![CheckPullRequest {
                number: 32,
                base: CheckPullRequestBase {
                    repo: CheckRepo { id: 7 }
                }
            }],
            suite.pull_requests
        );
    }

    #[test]
    fn test_check_hook_failed() {
        let check = |conclusion: Option<&str>| CheckHook {
            head_sha: "ffff0000".into(),
            conclusion: conclusion.map(String::from),
            pull_requests: vec![],
        };
        assert!(check(Some("failure")).failed());
        assert!(check(Some("timed_out")).failed());
        assert!(check(Some("startup_failure")).failed());
        assert!(!check(Some("success")).failed());
        assert!(!check(Some("cancelled")).failed());
        assert!(!check(None).failed());
    }
}
