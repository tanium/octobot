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
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct User {
    pub login: Option<String>,
}

impl User {
    pub fn new(login: &str) -> User {
        User { login: Some(login.to_string()) }
    }

    pub fn login(&self) -> &str {
        if let Some(ref login) = self.login {
            login
        } else {
            ""
        }
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
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct PullRequest {
    pub title: String,
    pub number: u32,
    pub html_url: String,
    pub state: String,
    pub user: User,
    pub merged: Option<bool>,
    pub merge_commit_sha: Option<String>,
    pub assignees: Vec<User>,
    pub head: BranchRef,
    pub base: BranchRef,
}

impl PullRequest {
    pub fn is_merged(&self) -> bool {
        self.merged.unwrap_or(false)
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Issue {
    pub html_url: String,
    pub title: String,
    pub user: User,
    pub assignees: Vec<User>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Label {
    pub name: String,
}


#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Review {
    pub state: String,
    pub body: Option<String>,
    pub html_url: String,
    pub user: User,
}

impl Review {
    pub fn body(&self) -> &str {
        match self.body {
            Some(ref body) => body,
            None => "",
        }
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

impl Comment {
    pub fn body(&self) -> &str {
        match self.body {
            Some(ref body) => body,
            None => "",
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct AssignResponse {
    pub assignees: Vec<User>,
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
}
