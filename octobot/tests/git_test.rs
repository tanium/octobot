mod git_helper;

use git_helper::temp_git::TempGit;

#[test]
fn test_current_branch() {
    let git = TempGit::new();

    assert_eq!("master", git.git.current_branch().unwrap());
    git.run_git(&["checkout", "-b", "other-branch"]);
    assert_eq!("other-branch", git.git.current_branch().unwrap());
}

#[test]
fn test_has_branch() {
    let git = TempGit::new();

    assert!(
        git.git.has_branch("master").unwrap(),
        "should have master branch"
    );

    git.run_git(&["branch", "falcon"]);
    git.run_git(&["branch", "eagle"]);

    assert!(
        git.git.has_branch("falcon").unwrap(),
        "should have falcon branch"
    );
    assert!(
        git.git.has_branch("eagle").unwrap(),
        "should have eagle branch"
    );
    assert!(
        !git.git.has_branch("eagles").unwrap(),
        "should NOT have eagles branch"
    );
}

#[test]
fn test_has_remote_branch() {
    let git = TempGit::new();

    git.run_git(&["checkout", "-b", "release/1.0.1"]);
    git.run_git(&["push", "origin", "release/1.0.1"]);

    assert!(git.git.has_remote_branch("release/1.0.1").unwrap());
    assert!(!git.git.has_remote_branch("release/1.0").unwrap());
}

#[test]
fn test_does_branch_contain() {
    let git = TempGit::new();

    let commit = git.run_git(&["rev-parse", "HEAD"]);

    git.run_git(&["branch", "falcon"]);
    git.run_git(&["branch", "eagle"]);

    assert!(
        git.git.does_branch_contain(&commit, "master").unwrap(),
        "master should contain commit"
    );
    assert!(
        git.git.does_branch_contain(&commit, "eagle").unwrap(),
        "eagle should contain commit"
    );
    assert!(
        git.git.does_branch_contain(&commit, "falcon").unwrap(),
        "falcon should contain commit"
    );

    git.run_git(&["checkout", "-b", "stallion"]);
    git.add_repo_file(
        "horses.txt",
        "Stallion\nSteed\nMustang\n",
        "Horses and stuff",
    );

    let commit2 = git.run_git(&["rev-parse", "HEAD"]);
    assert!(
        git.git.does_branch_contain(&commit, "stallion").unwrap(),
        "stallion should contain commit"
    );
    assert!(
        git.git.does_branch_contain(&commit2, "stallion").unwrap(),
        "stallion should contain commit2"
    );

    assert!(
        !git.git.does_branch_contain(&commit2, "master").unwrap(),
        "master should not contain commit2"
    );
    assert!(
        !git.git.does_branch_contain(&commit2, "eagle").unwrap(),
        "eagle should not contain commit2"
    );
    assert!(
        !git.git.does_branch_contain(&commit2, "falcon").unwrap(),
        "falcon should not contain commit2"
    );
}

#[test]
fn test_find_base_commit() {
    let git = TempGit::new();

    let base_commit = git.run_git(&["rev-parse", "HEAD"]);

    git.run_git(&["checkout", "-b", "falcon"]);
    git.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");
    git.add_repo_file("new-zealand-falcon.txt", "New Zealand", "falcons 2");
    git.add_repo_file("peregrine-falcon.txt", "Peregrine", "falcons 3");
    git.run_git(&["push", "origin", "falcon"]);
    let falcon_commit = git.run_git(&["rev-parse", "HEAD"]);

    git.run_git(&["checkout", "-b", "horses", "master"]);
    git.add_repo_file(
        "horses.txt",
        "Stallion\nSteed\nMustang\n",
        "Horses and stuff",
    );
    git.run_git(&["push", "origin", "horses"]);
    let horses_commit = git.run_git(&["rev-parse", "HEAD"]);

    git.reclone();

    // now change master!
    git.run_git(&["checkout", "master"]);
    git.add_repo_file("foo1.txt", "", "some other commit 1");
    git.add_repo_file("foo2.txt", "", "some other commit 2");
    let new_base_commit = git.run_git(&["rev-parse", "HEAD"]);
    git.run_git(&["push"]);

    // multiple commits back
    assert_eq!(
        base_commit,
        git.git
            .find_base_branch_commit(&falcon_commit, "master")
            .unwrap()
    );
    assert_eq!(
        base_commit,
        git.git
            .find_base_branch_commit("origin/falcon", "master")
            .unwrap()
    );

    // single commit back
    assert_eq!(
        base_commit,
        git.git
            .find_base_branch_commit(&horses_commit, "master")
            .unwrap()
    );
    assert_eq!(
        base_commit,
        git.git
            .find_base_branch_commit("origin/horses", "master")
            .unwrap()
    );

    // now rebase falcon!
    git.run_git(&["checkout", "falcon"]);
    git.run_git(&["rebase", "master"]);
    git.run_git(&["push", "-f"]);
    let new_falcon_commit = git.run_git(&["rev-parse", "HEAD"]);
    assert_eq!(
        base_commit,
        git.git
            .find_base_branch_commit(&falcon_commit, "master")
            .unwrap()
    );
    assert_eq!(
        new_base_commit,
        git.git
            .find_base_branch_commit(&new_falcon_commit, "master")
            .unwrap()
    );
    assert_eq!(
        new_base_commit,
        git.git.find_base_branch_commit("falcon", "master").unwrap()
    );

    // now force-push and reclone to clear reflog
    git.run_git(&["push", "-f"]);
    git.reclone();

    // get a local 'falcon' reference
    git.run_git(&["checkout", "falcon"]);
    git.run_git(&["checkout", "master"]);
    let new_falcon_commit = git.run_git(&["rev-parse", "HEAD"]);
    assert_eq!(
        new_base_commit,
        git.git
            .find_base_branch_commit(&new_falcon_commit, "master")
            .unwrap()
    );
    assert_eq!(
        new_base_commit,
        git.git.find_base_branch_commit("falcon", "master").unwrap()
    );

    // try rewriting the master branch. should stay the same...
    git.run_git(&[
        "commit",
        "--amend",
        "-m",
        "some other commit 2 -- muaha. re-written!",
    ]);
    git.run_git(&["push", "-f"]);
    let some_commit_1 = git.run_git(&["rev-parse", "HEAD^1"]);
    // --fork-point here knows that the base actually new_base_commit.
    assert_eq!(
        new_base_commit,
        git.git
            .find_base_branch_commit(&new_falcon_commit, "master")
            .unwrap()
    );

    // recloning again, reflog is gone, --fork-point would now fail.
    // regular merge-base is now be the one commit before the rewritten one.
    git.reclone();
    assert_eq!(
        some_commit_1,
        git.git
            .find_base_branch_commit(&new_falcon_commit, "master")
            .unwrap()
    );
}

#[test]
fn test_checkout_branch_new_local_branch() {
    let git = TempGit::new();

    let base_commit = git.run_git(&["rev-parse", "HEAD"]);

    git.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");
    let new_commit = git.run_git(&["rev-parse", "HEAD"]);

    git.run_git(&["push"]);
    git.run_git(&["reset", "--hard", &base_commit]);

    git.git
        .checkout_branch("some-new-branch", "origin/master")
        .unwrap();
    let now_commit = git.run_git(&["rev-parse", "HEAD"]);

    assert_eq!(now_commit, new_commit);
    assert_eq!("some-new-branch", git.git.current_branch().unwrap());
}

#[test]
fn test_checkout_branch_with_ref() {
    let git = TempGit::new();

    let base_commit = git.run_git(&["rev-parse", "HEAD"]);
    git.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");

    git.git
        .checkout_branch("some-new-branch", &base_commit)
        .unwrap();
    let now_commit = git.run_git(&["rev-parse", "HEAD"]);

    assert_eq!(now_commit, base_commit);
    assert_eq!("some-new-branch", git.git.current_branch().unwrap());
}

#[test]
fn test_checkout_branch_already_checked_out() {
    let git = TempGit::new();

    let base_commit = git.run_git(&["rev-parse", "HEAD"]);

    git.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");
    let new_commit = git.run_git(&["rev-parse", "HEAD"]);

    git.run_git(&["push"]);
    git.run_git(&["reset", "--hard", &base_commit]);

    git.git.checkout_branch("master", "origin/master").unwrap();
    let now_commit = git.run_git(&["rev-parse", "HEAD"]);

    assert_eq!(now_commit, new_commit);
    assert_eq!("master", git.git.current_branch().unwrap());
}

#[test]
fn test_checkout_branch_already_exists() {
    let git = TempGit::new();

    let base_commit = git.run_git(&["rev-parse", "HEAD"]);
    git.run_git(&["branch", "the-branch"]);

    git.add_repo_file("prarie-falcon.txt", "Prarie", "falcons 1");
    let new_commit = git.run_git(&["rev-parse", "HEAD"]);
    git.run_git(&["push"]);

    git.run_git(&["reset", "--hard", &base_commit]);

    git.git
        .checkout_branch("the-branch", "origin/master")
        .unwrap();
    let now_commit = git.run_git(&["rev-parse", "HEAD"]);

    assert_eq!(now_commit, new_commit);
    assert_eq!("the-branch", git.git.current_branch().unwrap());
}

#[test]
fn test_get_commit_desc() {
    let git = TempGit::new();

    git.run_git(&["commit", "--amend", "-m", "I have just a subject"]);
    assert_eq!(
        ("I have just a subject".into(), String::new()),
        git.git.get_commit_desc("HEAD").unwrap()
    );

    git.run_git(&[
        "commit",
        "--amend",
        "-m",
        "I have a subject\n\nAnd I have a body",
    ]);
    assert_eq!(
        ("I have a subject".into(), "And I have a body".into()),
        git.git.get_commit_desc("HEAD").unwrap()
    );

    git.run_git(&[
        "commit",
        "--amend",
        "-m",
        "I have a subject\nAnd I forgot to skip a line",
    ]);
    assert_eq!(
        (
            "I have a subject".into(),
            "And I forgot to skip a line".into(),
        ),
        git.git.get_commit_desc("HEAD").unwrap()
    );
}
