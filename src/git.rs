
pub trait Git: Send + Sync {
    fn create_merge_pr(&self);

    fn get_pr_labels(&self);
}

unsafe impl Send for GitOctocat {}
unsafe impl Sync for GitOctocat {}


#[derive(Clone)]
pub struct GitOctocat;

impl Git for GitOctocat {
    fn create_merge_pr(&self) {}

    fn get_pr_labels(&self) {}
}
