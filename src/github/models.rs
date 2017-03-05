use url::Url;

// An incomplete container for all the kinds of events that we care about.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct HookBody {
    pub repository: Repo,
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
            repository: Repo::new(),
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

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Repo {
    pub html_url: String,
    pub full_name: String,
    pub name: String,
    pub owner: User,
}

impl Repo {
    pub fn new() -> Repo {
        Repo {
            html_url: String::new(),
            full_name: String::new(),
            name: String::new(),
            owner: User::new(""),
        }
    }

    pub fn parse(html_url: &str) -> Result<Repo, String> {
        match Url::parse(html_url) {
            Ok(url) => {
                let segments: Vec<&str> = match url.path_segments() {
                    Some(s) => s.filter(|p| p.len() > 0).collect(),
                    None => return Err(format!("No path segments in URL")),
                };
                if segments.len() != 2 {
                    return Err(format!("Expectd only two path segments!"));
                }

                let user = segments[0];
                let repo = segments[1];

                Ok(Repo {
                    html_url: html_url.to_string(),
                    full_name: format!("{}/{}", user, repo),
                    name: repo.to_string(),
                    owner: User::new(user),
                })
            }
            Err(e) => return Err(format!("Error parsing url: {}", e)),
        }

    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
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

pub trait PullRequestLike {
    fn user(&self) -> &User;
    fn assignees(&self) -> Vec<User>;
    fn title(&self) -> &str;
    fn html_url(&self) -> &str;
    fn number(&self) -> u32;
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct PullRequest {
    pub title: String,
    pub body: String,
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
}

impl PullRequest {
    pub fn new() -> PullRequest {
        PullRequest {
            title: String::new(),
            body: String::new(),
            number: 0,
            html_url: String::new(),
            state: "open".into(),
            user: User::new(""),
            merged: None,
            merge_commit_sha: None,
            assignees: vec![],
            requested_reviewers: None,
            head: BranchRef::new(""),
            base: BranchRef::new(""),
        }
    }

    pub fn is_merged(&self) -> bool {
        self.merged.unwrap_or(false)
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
}

#[derive(Deserialize, Serialize, Clone, Debug)]
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
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Label {
    pub name: String,
}

impl Label {
    pub fn new(name: &str) -> Label {
        Label {
            name: name.into(),
        }
    }
}


pub trait CommentLike {
    fn user(&self) -> &User;
    fn body(&self) -> &str;
    fn html_url(&self) -> &str;
}


#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Review {
    pub state: String,
    pub body: Option<String>,
    pub html_url: String,
    pub user: User,
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


#[derive(Deserialize, Serialize, Clone, Debug)]
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

pub trait CommitLike {
    fn sha(&self) -> &str;
    fn html_url(&self) -> &str;
    fn message(&self) -> &str;
}

// the 'Commit' objects that come from a push event have a different format from
// the api that lists commits for a PR
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct PushCommit {
    pub id: String,
    pub tree_id: String,
    pub message: String,
    pub url: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Commit {
    pub sha: String,
    pub html_url: String,
    pub commit: CommitDetails,
    pub author: Option<User>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
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
            commit: CommitDetails {
                message: String::new(),
            }
        }
    }

    pub fn short_hash(commit: &CommitLike) -> &str {
        Commit::short_hash_str(commit.sha())
    }

    pub fn short_hash_str(hash: &str) -> &str {
        if hash.len() < 7 {
            hash
        } else {
            &hash[0..7]
        }
    }

    pub fn title(commit: &CommitLike) -> String {
        commit.message().lines().next().unwrap_or("").into()
    }

    pub fn body(commit: &CommitLike) -> String {
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

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct AssignResponse {
    pub assignees: Vec<User>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Status {
    pub state: String,
    pub target_url: Option<String>,
    pub context: Option<String>,
    pub description: Option<String>,
    pub creator: Option<User>,
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

        let all_users = vec![User::new("user1"), User::new("user2"),
                             User::new("userC"), User::new("userD")];

        assert_eq!(all_users, (&pr).assignees());
    }
}
