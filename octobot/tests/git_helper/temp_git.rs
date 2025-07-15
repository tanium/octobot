use std;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;

use tempfile::{tempdir, TempDir};

use octobot_ops::git::Git;

pub struct TempGit {
    _dir: TempDir,
    pub git: Git,
    pub repo_dir: PathBuf,
}

impl TempGit {
    pub fn new() -> TempGit {
        let home = tempdir().expect("create fake home dir for configs").keep();
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", &home);

        // These will interfere with tests run locally
        std::env::remove_var("GIT_AUTHOR_NAME");
        std::env::remove_var("GIT_AUTHOR_EMAIL");

        let dir = tempdir().expect("create temp dir for git_test.rs");

        let repo_dir = dir.path().join("repo");
        let remote_dir = dir.path().join("remote");
        std::fs::create_dir(&remote_dir).expect("create remote dir");

        let git = Git::new("the-host", "the-token", &repo_dir);
        let remote_git = Git::new("the-host", "the-token", &remote_dir);

        remote_git.run(&["--bare", "init"]).expect("init base repo");

        let test = TempGit {
            _dir: dir,
            git,
            repo_dir,
        };

        test.reclone();

        // add an initial commit to start with.
        test.add_repo_file("README.md", "# TempGit\n\n", "Initial README commit!");
        test.run_git(&["push"]);

        test
    }

    pub fn user_name(&self) -> &str {
        "Test User"
    }

    pub fn user_email(&self) -> &str {
        "testy@octobot.com"
    }

    pub fn run_git(&self, args: &[&str]) -> String {
        let user = format!("user.name={}", self.user_name());
        let email = format!("user.email={}", self.user_email());
        let mut full_args = vec!["-c", &user, "-c", &email];
        full_args.extend(args.iter());
        self.git
            .run(&full_args)
            .unwrap_or_else(|_| panic!("Failed running git: `{:?}`", args))
    }

    pub fn reclone(&self) {
        if self.repo_dir.exists() {
            std::fs::remove_dir_all(&self.repo_dir).expect("remove clone dir");
        }
        if !self.repo_dir.exists() {
            std::fs::create_dir(&self.repo_dir).expect("create clone dir");
        }

        self.git
            .run(&["clone", "../remote", "."])
            .expect("clone from bare repo");
        self.git
            .run(&["config", "commit.gpgsign", "false"])
            .expect("turn off gpg signing");
    }

    pub fn add_repo_file(&self, path: &str, contents: &str, msg: &str) {
        self.write_file(path, contents);
        self.run_git(&["add", path]);
        self.run_git(&["commit", "-a", "-m", msg]);
    }

    fn write_file(&self, path: &str, contents: &str) {
        let path = self.repo_dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|_| panic!("create dir '{:?}'", parent));
        }

        let mut file = File::create(path).expect("create file");
        file.write_all(contents.as_bytes())
            .expect("write contents to file");
    }

    pub fn read_file(&self, path: &str) -> String {
        let path = self.repo_dir.join(path);
        let mut f =
            std::fs::File::open(&path).unwrap_or_else(|_| panic!("unable to open file {:?}", path));
        let mut contents = String::new();
        f.read_to_string(&mut contents)
            .unwrap_or_else(|_| panic!("error reading file {:?}", path));
        contents
    }
}
