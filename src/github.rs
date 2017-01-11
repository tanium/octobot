
pub struct GithubUser {
    pub login: String,
}

pub struct GithubRepo {
    pub html_url: String,
    pub full_name: String,
    pub owner: GithubUser,
}
