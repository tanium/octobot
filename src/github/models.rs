use url::Url;

// An incomplete container for all the kinds of events that we care about.
#[derive(RustcDecodable, RustcEncodable, Clone, Debug)]
pub struct HookBody {
    pub repository: Repo,
    pub sender: User,

    pub action: Option<String>,
    pub issue: Option<Issue>,
    pub comment: Option<Comment>,
    pub pull_request: Option<PullRequest>,
    pub review: Option<Review>,
}

#[derive(RustcDecodable, RustcEncodable, Clone, Debug)]
pub struct User {
    pub login: String,
}

impl User {
    pub fn new(login: &str) -> User {
        User { login: login.to_string() }
    }
}

#[derive(RustcDecodable, RustcEncodable, Clone, Debug)]
pub struct Repo {
    pub html_url: String,
    pub full_name: String,
    pub owner: User,
}

impl Repo {
    pub fn new() -> Repo {
        Repo {
            html_url: String::new(),
            full_name: String::new(),
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
                    owner: User::new(user),
                })
            },
            Err(e) => return Err(format!("Error parsing url: {}", e))
        }

    }

}

#[derive(RustcDecodable, RustcEncodable, Clone, Debug)]
pub struct PullRequest {
    pub title: String,
    pub number: i32,
    pub html_url: String,
    pub state: String,
    pub user: User,
    pub merged: Option<bool>,
    pub merge_commit_sha: Option<String>,
    pub assignees: Vec<User>,
}

#[derive(RustcDecodable, RustcEncodable, Clone, Debug)]
pub struct Issue {
    pub html_url: String,
    pub title: String,
    pub user: User,
    pub assignees: Vec<User>,
}

#[derive(RustcDecodable, RustcEncodable, Clone, Debug)]
pub struct Review {
    pub state: String,
    pub body: String,
    pub html_url: String,
    pub user: User,
}

#[derive(RustcDecodable, RustcEncodable, Clone, Debug)]
pub struct Comment {
    pub commit_id: String,
    pub path: Option<String>,
    pub body: String,
    pub html_url: String,
    pub user: User,
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
        assert_eq!("users", repo.owner.login);
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
