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
        std::fs::create_dir(&remote_dir).expect("create remote dir");

        let git = Git::new("the-host", "the-token", &repo_dir);
        let remote_git = Git::new("the-host", "the-token", &remote_dir);

        remote_git.run(&["--bare", "init"]).expect("init base repo");

        let test = GitTest {
            _dir: dir,
            git: git,
            repo_dir: repo_dir,
        };

        test.reclone();

        // add an initial commit to start with.
        test.add_repo_file("README.md", "# GitTest\n\n", "Initial README commit!");
        test.run_git(&["push"]);

        test
    }

    fn run_git(&self, args: &[&str]) -> String {
        self.git.run(args).expect(&format!("Failed running git: `{:?}`", args))
    }

    fn reclone(&self) {
        if self.repo_dir.exists() {
            std::fs::remove_dir_all(&self.repo_dir).expect("remove clone dir");
        }
        if !self.repo_dir.exists() {
            std::fs::create_dir(&self.repo_dir).expect("create clone dir");
        }

        self.git.run(&["clone", "../remote", "."]).expect("clone from bare repo");
        self.git.run(&["config", "commit.gpgsign", "false"]).expect("turn off gpg signing");
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

    assert_eq!(Ok("master".into()), test.git.current_branch());
    test.run_git(&["checkout", "-b", "other-branch"]);
    assert_eq!(Ok("other-branch".into()), test.git.current_branch());
}

#[test]
fn test_has_branch() {
    let test = GitTest::new();

    assert_eq!(Ok(true), test.git.has_branch("master"), "should have master branch");

    test.run_git(&["branch", "falcon"]);
    test.run_git(&["branch", "eagle"]);

    assert_eq!(Ok(true), test.git.has_branch("falcon"), "should have falcon branch");
    assert_eq!(Ok(true), test.git.has_branch("eagle"), "should have eagle branch");
    assert_eq!(Ok(false), test.git.has_branch("eagles"), "should NOT have eagles branch");
}

#[test]
fn test_does_branch_contain() {
    let test = GitTest::new();

    let commit = test.run_git(&["rev-parse", "HEAD"]);

    test.run_git(&["branch", "falcon"]);
    test.run_git(&["branch", "eagle"]);

    assert_eq!(Ok(true), test.git.does_branch_contain(&commit, "master"), "master should contain commit");
    assert_eq!(Ok(true), test.git.does_branch_contain(&commit, "eagle"), "eagle should contain commit");
    assert_eq!(Ok(true), test.git.does_branch_contain(&commit, "falcon"), "falcon should contain commit");

    test.run_git(&["checkout", "-b", "stallion"]);
    test.add_repo_file("horses.txt", "Stallion\nSteed\nMustang\n", "Horses and stuff");

    let commit2 = test.run_git(&["rev-parse", "HEAD"]);
    assert_eq!(Ok(true), test.git.does_branch_contain(&commit, "stallion"), "stallion should contain commit");
    assert_eq!(Ok(true), test.git.does_branch_contain(&commit2, "stallion"), "stallion should contain commit2");

    assert_eq!(Ok(false), test.git.does_branch_contain(&commit2, "master"), "master should not contain commit2");
    assert_eq!(Ok(false), test.git.does_branch_contain(&commit2, "eagle"), "eagle should not contain commit2");
    assert_eq!(Ok(false), test.git.does_branch_contain(&commit2, "falcon"), "falcon should not contain commit2");
}

#[test]
fn test_find_base_commit() {
    let test = GitTest::new();

    let base_commit = test.run_git(&["rev-parse", "HEAD"]);

    test.run_git(&["checkout", "-b", "falcon"]);
    test.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");
    test.add_repo_file("new-zealand-falcon.txt", "New Zealand", "falcons 2");
    test.add_repo_file("peregrine-falcon.txt", "Peregrine", "falcons 3");
    test.run_git(&["push", "origin", "falcon"]);
    let falcon_commit = test.run_git(&["rev-parse", "HEAD"]);

    test.run_git(&["checkout", "-b", "horses", "master"]);
    test.add_repo_file("horses.txt", "Stallion\nSteed\nMustang\n", "Horses and stuff");
    test.run_git(&["push", "origin", "horses"]);
    let horses_commit = test.run_git(&["rev-parse", "HEAD"]);

    test.reclone();

    // now change master!
    test.run_git(&["checkout", "master"]);
    test.add_repo_file("foo1.txt", "", "some other commit 1");
    test.add_repo_file("foo2.txt", "", "some other commit 2");
    let new_base_commit = test.run_git(&["rev-parse", "HEAD"]);
    test.run_git(&["push"]);

    // multiple commits back
    assert_eq!(Ok(base_commit.clone()), test.git.find_base_branch_commit(&falcon_commit, "master"));
    assert_eq!(Ok(base_commit.clone()), test.git.find_base_branch_commit("origin/falcon", "master"));

    // single commit back
    assert_eq!(Ok(base_commit.clone()), test.git.find_base_branch_commit(&horses_commit, "master"));
    assert_eq!(Ok(base_commit.clone()), test.git.find_base_branch_commit("origin/horses", "master"));

    // now rebase falcon!
    test.run_git(&["checkout", "falcon"]);
    test.run_git(&["rebase", "master"]);
    test.run_git(&["push", "-f"]);
    let new_falcon_commit = test.run_git(&["rev-parse", "HEAD"]);
    assert_eq!(Ok(base_commit.clone()), test.git.find_base_branch_commit(&falcon_commit, "master"));
    assert_eq!(Ok(new_base_commit.clone()), test.git.find_base_branch_commit(&new_falcon_commit, "master"));
    assert_eq!(Ok(new_base_commit.clone()), test.git.find_base_branch_commit("falcon", "master"));

    // now force-push and reclone to clear reflog
    test.run_git(&["push", "-f"]);
    test.reclone();

    // get a local 'falcon' reference
    test.run_git(&["checkout", "falcon"]);
    test.run_git(&["checkout", "master"]);
    let new_falcon_commit = test.run_git(&["rev-parse", "HEAD"]);
    assert_eq!(Ok(new_base_commit.clone()), test.git.find_base_branch_commit(&new_falcon_commit, "master"));
    assert_eq!(Ok(new_base_commit.clone()), test.git.find_base_branch_commit("falcon", "master"));

    // try rewriting the master branch. should stay the same...
    test.run_git(&["commit", "--amend", "-m", "some other commit 2 -- muaha. re-written!"]);
    test.run_git(&["push", "-f"]);
    let some_commit_1 = test.run_git(&["rev-parse", "HEAD^1"]);
    // --fork-point here knows that the base actually new_base_commit.
    assert_eq!(Ok(new_base_commit.clone()), test.git.find_base_branch_commit(&new_falcon_commit, "master"));

    // recloning again, reflog is gone, --fork-point would now fail.
    // regular merge-base is now be the one commit before the rewritten one.
    test.reclone();
    assert_eq!(Ok(some_commit_1.clone()), test.git.find_base_branch_commit(&new_falcon_commit, "master"));
}

#[test]
fn test_checkout_branch_new_local_branch() {
    let test = GitTest::new();

    let base_commit = test.run_git(&["rev-parse", "HEAD"]);

    test.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");
    let new_commit = test.run_git(&["rev-parse", "HEAD"]);

    test.run_git(&["push"]);
    test.run_git(&["reset", "--hard", &base_commit]);

    assert_eq!(Ok(()), test.git.checkout_branch("some-new-branch", "origin/master"));
    let now_commit = test.run_git(&["rev-parse", "HEAD"]);

    assert_eq!(now_commit, new_commit);
    assert_eq!(Ok("some-new-branch".into()), test.git.current_branch());
}

#[test]
fn test_checkout_branch_with_ref() {
    let test = GitTest::new();

    let base_commit = test.run_git(&["rev-parse", "HEAD"]);
    test.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");

    assert_eq!(Ok(()), test.git.checkout_branch("some-new-branch", &base_commit));
    let now_commit = test.run_git(&["rev-parse", "HEAD"]);

    assert_eq!(now_commit, base_commit);
    assert_eq!(Ok("some-new-branch".into()), test.git.current_branch());
}

#[test]
fn test_checkout_branch_already_checked_out() {
    let test = GitTest::new();

    let base_commit = test.run_git(&["rev-parse", "HEAD"]);

    test.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");
    let new_commit = test.run_git(&["rev-parse", "HEAD"]);

    test.run_git(&["push"]);
    test.run_git(&["reset", "--hard", &base_commit]);

    assert_eq!(Ok(()), test.git.checkout_branch("master", "origin/master"));
    let now_commit = test.run_git(&["rev-parse", "HEAD"]);

    assert_eq!(now_commit, new_commit);
    assert_eq!(Ok("master".into()), test.git.current_branch());
}

#[test]
fn test_checkout_branch_already_exists() {
    let test = GitTest::new();

    let base_commit = test.run_git(&["rev-parse", "HEAD"]);
    test.run_git(&["branch", "the-branch"]);

    test.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");
    let new_commit = test.run_git(&["rev-parse", "HEAD"]);
    test.run_git(&["push"]);

    test.run_git(&["reset", "--hard", &base_commit]);

    assert_eq!(Ok(()), test.git.checkout_branch("the-branch", "origin/master"));
    let now_commit = test.run_git(&["rev-parse", "HEAD"]);

    assert_eq!(now_commit, new_commit);
    assert_eq!(Ok("the-branch".into()), test.git.current_branch());
}

#[test]
fn test_get_commit_desc() {
    let test = GitTest::new();

    test.run_git(&["commit", "--amend", "-m", "I have just a subject"]);
    assert_eq!(Ok(("I have just a subject".into(), String::new())), test.git.get_commit_desc("HEAD"));

    test.run_git(&["commit", "--amend", "-m", "I have a subject\n\nAnd I have a body"]);
    assert_eq!(Ok(("I have a subject".into(), "And I have a body".into())), test.git.get_commit_desc("HEAD"));

    test.run_git(
        &["commit", "--amend", "-m", "I have a subject\nAnd I forgot to skip a line"],
    );
    assert_eq!(Ok(("I have a subject".into(), "And I forgot to skip a line".into())), test.git.get_commit_desc("HEAD"));
}
