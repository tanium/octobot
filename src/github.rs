use rustc_serialize::json;

// An incomplete container for all the kinds of events that we care about.
#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct HookBody {
    pub repository: Repo,
    pub sender: User,

    pub action: Option<String>,
    pub issue: Option<Issue>,
    pub comment: Option<Comment>,
    pub pull_request: Option<PullRequest>,
    pub review: Option<Review>,
}

#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct User {
    pub login: String,
}

#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct Repo {
    pub html_url: String,
    pub full_name: String,
    pub owner: User,
}

#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct PullRequest {
    pub title: String,
    pub number: i32,
    pub html_url: String,
    pub state: String,
    pub user: User,
    pub merge_commit_sha: Option<String>,
    pub assignees: Vec<User>,
}

#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct Issue {
    pub html_url: String,
    pub title: String,
    pub user: User,
    pub assignees: Vec<User>,
}

#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct Review {
    pub state: String,
    pub body: String,
    pub html_url: String,
    pub user: User,
}

#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct Comment {
    pub commit_id: String,
    pub path: Option<String>,
    pub body: String,
    pub html_url: String,
    pub user: User,
}
