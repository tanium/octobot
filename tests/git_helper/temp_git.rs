use std;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use tempdir::TempDir;

use octobot::git::Git;

pub struct TempGit {
    _dir: TempDir,
    pub git: Git,
    pub repo_dir: PathBuf,
}

impl TempGit {
    pub fn new() -> TempGit {
        let dir = TempDir::new("git_test.rs").expect("create temp dir for git_test.rs");

        let repo_dir = dir.path().join("repo");
        let remote_dir = dir.path().join("remote");
        std::fs::create_dir(&remote_dir).expect("create remote dir");

        let git = Git::new("the-host", "the-token", &repo_dir);
        let remote_git = Git::new("the-host", "the-token", &remote_dir);

        remote_git.run(&["--bare", "init"]).expect("init base repo");

        let test = TempGit {
            _dir: dir,
            git: git,
            repo_dir: repo_dir,
        };

        test.reclone();

        // add an initial commit to start with.
        test.add_repo_file("README.md", "# TempGit\n\n", "Initial README commit!");
        test.run_git(&["push"]);

        test
    }

    pub fn run_git(&self, args: &[&str]) -> String {
        self.git.run(args).expect(&format!("Failed running git: `{:?}`", args))
    }

    pub fn reclone(&self) {
        if self.repo_dir.exists() {
            std::fs::remove_dir_all(&self.repo_dir).expect("remove clone dir");
        }
        if !self.repo_dir.exists() {
            std::fs::create_dir(&self.repo_dir).expect("create clone dir");
        }

        self.git.run(&["clone", "../remote", "."]).expect("clone from bare repo");
        self.git.run(&["config", "commit.gpgsign", "false"]).expect("turn off gpg signing");
    }

    pub fn add_repo_file(&self, path: &str, contents: &str, msg: &str) {
        self.write_file(path, contents);
        self.run_git(&["add", path]);
        self.run_git(&["commit", "-a", "-m", msg]);
    }

    fn write_file(&self, path: &str, contents: &str) {
        let path = self.repo_dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect(&format!("create dir '{:?}'", parent));
        }

        let mut file = File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write contents to file");
    }
}
