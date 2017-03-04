extern crate octobot;
extern crate tempdir;

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use octobot::git::Git;
use tempdir::TempDir;

struct GitTest {
    _dir: TempDir,
    git: Git,
    repo_dir: PathBuf,
}

impl GitTest {
    fn new() -> GitTest {
        let dir = TempDir::new("git_test.rs").expect("create temp dir for git_test.rs");

        let repo_dir = dir.path().join("repo");
        let remote_dir = dir.path().join("remote");
        std::fs::create_dir(&repo_dir).expect("create repo dir");
        std::fs::create_dir(&remote_dir).expect("create remote dir");

        let git = Git::new("the-host", "the-token", &repo_dir);
        let remote_git = Git::new("the-host", "the-token", &remote_dir);

        remote_git.run(&["--bare", "init"]).unwrap();
        git.run(&["clone", "../remote", "."]).unwrap();

        let test = GitTest {
            _dir: dir,
            git: git,
            repo_dir: repo_dir,
        };

        // add an initial commit to start with.
        test.add_repo_file("README.md", "# GitTest\n\n", "Initial README commit!");
        test.run_git(&["push"]);

        test
    }

    fn run_git(&self, args: &[&str]) -> String {
        self.git.run(args).expect(&format!("Failed running git: `{:?}`", args))
    }

    fn add_repo_file(&self, path: &str, contents: &str, msg: &str) {
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

#[test]
fn test_current_branch() {
    let test = GitTest::new();

    assert_eq!("master", test.git.current_branch().unwrap());
    test.git.run(&["checkout", "-b", "other-branch"]).unwrap();
    assert_eq!("other-branch", test.git.current_branch().unwrap());
}

#[test]
fn test_has_branch() {
    let test = GitTest::new();

    assert!(test.git.has_branch("master").unwrap(), "should have master branch");

    test.run_git(&["branch", "falcon"]);
    test.run_git(&["branch", "eagle"]);

    assert!(test.git.has_branch("falcon").unwrap(), "should have falcon branch");
    assert!(test.git.has_branch("eagle").unwrap(), "should have eagle branch");
    assert!(!test.git.has_branch("eagles").unwrap(), "should NOT have eagles branch");
}

#[test]
fn test_does_branch_contain() {
    let test = GitTest::new();

    let commit = test.run_git(&["rev-parse", "HEAD"]);

    test.run_git(&["branch", "falcon"]);
    test.run_git(&["branch", "eagle"]);

    assert!(test.git.does_branch_contain(&commit, "master").unwrap(), "master should contain commit");
    assert!(test.git.does_branch_contain(&commit, "eagle").unwrap(), "eagle should contain commit");
    assert!(test.git.does_branch_contain(&commit, "falcon").unwrap(), "falcon should contain commit");

    test.run_git(&["checkout", "-b", "stallion"]);
    test.add_repo_file("horses.txt", "Stallion\nSteed\nMustang\n", "Horses and stuff");

    let commit2 = test.run_git(&["rev-parse", "HEAD"]);
    assert!(test.git.does_branch_contain(&commit, "stallion").unwrap(), "stallion should contain commit");
    assert!(test.git.does_branch_contain(&commit2, "stallion").unwrap(), "stallion should contain commit2");

    assert!(!test.git.does_branch_contain(&commit2, "master").unwrap(), "master should not contain commit2");
    assert!(!test.git.does_branch_contain(&commit2, "eagle").unwrap(), "eagle should not contain commit2");
    assert!(!test.git.does_branch_contain(&commit2, "falcon").unwrap(), "falcon should not contain commit2");
}
