use failure::format_err;
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
            ref_name: None,
            after: None,
            before: None,
            compare: None,
            forced: None,
            deleted: None,
            created: None,
            commits: None,
        }
    }

    pub fn ref_name(&self) -> &str {
        match self.ref_name {
            Some(ref v) => v,
            None => "",
        }
    }

    pub fn after(&self) -> &str {
        match self.after {
            Some(ref v) => v,
            None => "",
        }
    }

    pub fn before(&self) -> &str {
        match self.before {
            Some(ref v) => v,
            None => "",
        }
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
pub struct User {
    pub login: Option<String>,
    pub name: Option<String>,
}

impl User {
    pub fn new(login: &str) -> User {
        User {
            login: Some(login.to_string()),
            name: Some(login.to_string()),
        }
    }

    pub fn login(&self) -> &str {
        if let Some(ref login) = self.login {
            login
        } else if let Some(ref name) = self.name {
            name
        } else {
            ""
        }
    }
}

impl PartialEq for User {
    fn eq(&self, other: &User) -> bool {
        self.login() == other.login()
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Repo {
    pub html_url: String,
    pub full_name: String,
    pub name: String,
    pub owner: User,
    pub archived: Option<bool>,
}

impl Repo {
    pub fn new() -> Repo {
        Repo {
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
            Some(s) => s.filter(|p| p.len() > 0).collect(),
            None => return Err(format_err!("No path segments in URL")),
        };
        if segments.len() != 2 {
            return Err(format_err!("Expectd only two path segments!"));
        }

        let user = segments[0];
        let repo = segments[1];

        Ok(Repo {
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

pub trait PullRequestLike : Send + Sync {
    fn user(&self) -> &User;
    fn assignees(&self) -> Vec<User>;
    fn title(&self) -> &str;
    fn html_url(&self) -> &str;
    fn number(&self) -> u32;
    fn has_commits(&self) -> bool;
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
    pub reviews: Option<Vec<Review>>,
    pub draft: Option<bool>,
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
            reviews: None,
            head: BranchRef::new(""),
            base: BranchRef::new(""),
            draft: None,
        }
    }

    pub fn is_merged(&self) -> bool {
        self.merged.unwrap_or(false)
    }

    pub fn is_draft(&self) -> bool {
        self.draft.unwrap_or(false) || self.title.to_lowercase().starts_with("wip:")
    }

    pub fn all_reviewers(&self) -> Vec<User> {
        let mut reviewers = vec![];
        if let Some(ref requested_reviewers) = self.requested_reviewers {
            reviewers.extend(requested_reviewers.clone().into_iter());
        }

        if let Some(ref reviews) = self.reviews {
            reviewers.extend(reviews.iter().map(|ref r| r.user.clone()));
        }

        reviewers
    }
}

impl<'a> PullRequestLike for &'a PullRequest {
    fn user(&self) -> &User {
        &self.user
    }

    fn assignees(&self) -> Vec<User> {
        let mut assignees = self.assignees.clone();
        if let Some(ref reviewers) = self.requested_reviewers {
            assignees.extend(reviewers.iter().map(|r| r.clone()));
        }
        if let Some(ref reviews) = self.reviews {
            assignees.extend(reviews.iter().map(|r| r.user.clone()));
        }
        assignees
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
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Issue {
    pub number: u32,
    pub html_url: String,
    pub title: String,
    pub user: User,
    pub assignees: Vec<User>,
}

impl<'a> PullRequestLike for &'a Issue {
    fn user(&self) -> &User {
        &self.user
    }

    fn assignees(&self) -> Vec<User> {
        self.assignees.clone()
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


pub trait CommentLike : Send + Sync {
    fn user(&self) -> &User;
    fn body(&self) -> &str;
    fn html_url(&self) -> &str;
}


#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Review {
    pub state: String,
    pub body: Option<String>,
    pub html_url: String,
    pub user: User,
}

impl Review {
    pub fn new(body: &str, user: User) -> Review {
        Review {
            state: "COMMENTED".into(),
            body: Some(body.into()),
            html_url: String::new(),
            user: user,
        }
    }
}

impl<'a> CommentLike for &'a Review {
    fn body(&self) -> &str {
        match self.body {
            Some(ref body) => body,
            None => "",
        }
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
    pub commit_id: Option<String>,
    pub path: Option<String>,
    pub body: Option<String>,
    pub html_url: String,
    pub user: User,
}

impl<'a> CommentLike for &'a Comment {
    fn body(&self) -> &str {
        match self.body {
            Some(ref body) => body,
            None => "",
        }
    }

    fn user(&self) -> &User {
        &self.user
    }

    fn html_url(&self) -> &str {
        &self.html_url
    }
}

pub trait CommitLike : Send + Sync {
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

impl<'a> CommitLike for &'a PushCommit {
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


impl<'a> CommitLike for &'a Commit {
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
            commit: CommitDetails { message: String::new() },
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
        let lines: Vec<&str> = commit.message().lines().skip(1).skip_while(|ref l| l.trim().len() == 0).collect();
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

        return false;
    }

    pub fn dismissed_review_id(&self) -> Option<u32> {
        match self.dismissed_review {
            Some(ref r) => Some(r.review_id),
            None => None,
        }
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

        return false;
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
            review_id: review_id,
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
            assert_eq!(false, body.forced());
            assert_eq!(false, body.created());
            assert_eq!(false, body.deleted());
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
            assert_eq!(true, body.forced());
        }

        {
            let mut body = HookBody::new();
            body.created = Some(true);
            assert_eq!(true, body.created());
        }
        {
            let mut body = HookBody::new();
            body.deleted = Some(true);
            assert_eq!(true, body.deleted());
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

        commit.commit.message = "4 Hello there\n\nand then some more\nwith\nmultiple\n\nlines".into();
        assert_eq!("4 Hello there", Commit::title(&commit));
        assert_eq!("and then some more\nwith\nmultiple\n\nlines", Commit::body(&commit));

        commit.commit.message = "5 Hello there\r\n\r\nmaybe also support\r\ncarriage\r\nreturns?".into();
        assert_eq!("5 Hello there", Commit::title(&commit));
        assert_eq!("maybe also support\ncarriage\nreturns?", Commit::body(&commit));
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

        assert_eq!(users, (&pr).assignees());

        let reviewers = vec![User::new("userC"), User::new("userD")];
        pr.requested_reviewers = Some(reviewers.clone());

        let all_users = vec![User::new("user1"), User::new("user2"), User::new("userC"), User::new("userD")];
        assert_eq!(all_users, (&pr).assignees());

        let reviews = vec![Review::new("i like it", User::new("userE")), Review::new("i like it", User::new("userF"))];
        pr.reviews = Some(reviews);

        let even_more_users = vec![
            User::new("user1"),
            User::new("user2"),
            User::new("userC"),
            User::new("userD"),
            User::new("userE"),
            User::new("userF"),
        ];
        assert_eq!(even_more_users, (&pr).assignees());
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
}
