extern crate octobot;

mod mocks;

use mocks::mock_github::MockGithub;

use octobot::force_push;
use octobot::github;
use octobot::diffs::DiffOfDiffs;

#[test]
fn test_force_push_identical() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;
    pr.head.ref_name = "the-pr-branch".into();

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok(DiffOfDiffs::new(&diff, &diff));

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: abcdef0, after: 1111abc: Identical diff post-rebase",
        Ok(()));

    github.mock_get_statuses("some-user", "some-repo", "abcdef0999999", Ok(vec![]));

    force_push::comment_force_push(diffs, vec![], &github, "some-user", "some-repo", &pr,
                                   "abcdef0999999", "1111abc9999999").unwrap();
}

#[test]
fn test_force_push_identical_with_statuses() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok(DiffOfDiffs::new(&diff, &diff));

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: the-bef, after: the-aft: Identical diff post-rebase",
        Ok(()));


    let statuses = vec![
        github::Status {
            state: "success".into(),
            target_url: Some("http://ci/build".into()),
            context: Some("ci/build".into()),
            description: Some("the desc".into()),
            creator: None,
        },
        github::Status {
            state: "failure".into(),
            target_url: None,
            context: Some("checks/cla".into()),
            description: None,
            creator: None,
        },
        github::Status {
            state: "error".into(),
            target_url: None,
            context: Some("checks/cla".into()), // duplicate context -- should be ignored
            description: None,
            creator: None,
        },
        github::Status {
            state: "pending".into(),
            target_url: None,
            context: Some("something/else".into()),
            description: None,
            creator: None,
        },
    ];

    github.mock_get_statuses("some-user", "some-repo", "the-before-hash", Ok(statuses));

    let new_status1 = github::Status {
        state: "success".into(),
        target_url: Some("http://ci/build".into()),
        context: Some("ci/build".into()),
        description: Some("the desc (reapplied by octobot)".into()),
        creator: None,
    };
    let new_status2 = github::Status {
        state: "failure".into(),
        target_url: None,
        context: Some("checks/cla".into()),
        description: Some("(reapplied by octobot)".into()),
        creator: None,
    };


    github.mock_create_status("some-user", "some-repo", "the-after-hash", &new_status1, Ok(()));
    github.mock_create_status("some-user", "some-repo", "the-after-hash", &new_status2, Ok(()));

    force_push::comment_force_push(diffs, vec!["ci/build".into(), "checks/cla".into()], &github,
                                   "some-user", "some-repo", &pr, "the-before-hash", "the-after-hash").unwrap();
}


#[test]
fn test_force_push_different() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let diffs = Ok(DiffOfDiffs::new("diff1", "diff2"));

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: abcdef0, after: 1111abc: Diff changed post-rebase",
        Ok(()));

    force_push::comment_force_push(diffs, vec![], &github, "some-user", "some-repo", &pr,
                                   "abcdef0999999", "1111abc9999999").unwrap();
}

#[test]
fn test_force_push_different_with_details() {
    let diff0 = r#"
diff --git a/src/diffs.rs b/src/diffs.rs
index 9c8643c..5aa6c73 100644
--- a/src/diffs.rs
+++ b/src/diffs.rs
@@ -1,3 +1,4 @@
+use std::cmp::max;
 use unidiff::{PatchSet, PatchedFile, Hunk, Line};

 #[derive(Debug)]
@@ -36,6 +37,16 @@ impl DiffOfDiffs {

         are_patch_sets_equal(patch0, patch1)
     }
+
+    pub fn different_patch_files(&self) -> Vec<PatchedFile> {
+        if let Some(ref patch0) = self.patch0 {
+            if let Some(ref patch1) = self.patch1 {
+                return different_patch_files(patch0, patch1)
+            }
+        }
+
+        vec![]
+    }
 }

 fn parse_diff(diff: &str) -> Option<PatchSet> {
diff --git a/src/force_push.rs b/src/force_push.rs
index 33667da..3503c28 100644
--- a/src/force_push.rs
+++ b/src/force_push.rs
@@ -29,10 +29,15 @@ pub fn comment_force_push(diffs: Result<DiffOfDiffs, String>,
                 comment += "Identical diff post-rebase";
                 identical_diff = true;
             } else {
-                // TODO: How to expose this diff -- maybe create a secret gist?
-                // But that may raise permissions concerns for users who can read octobot's gists,
-                // but perhaps not the original repo...
                 comment += "Diff changed post-rebase";
+                let different_files = diffs.different_patch_files();
+                if different_files.len() > 0 {
+                    comment += "\n\nChanged files:\n";
+                    for file in different_files {
+                        comment += &format!("* {}", file.path());
+                    }
+                }
+
                 identical_diff = false;
             }
         }, "#;

    let diff1 = r#"
diff --git a/src/diffs.rs b/src/diffs.rs
index 9c8643c..5aa6c73 100644
--- a/src/diffs.rs
+++ b/src/diffs.rs
@@ -1,3 +1,4 @@
+use std::cmp::max;
 use unidiff::{PatchSet, PatchedFile, Hunk, Line};

 #[derive(Debug)]
@@ -36,6 +37,16 @@ impl DiffOfDiffs {

         are_patch_sets_equal(patch0, patch1)
     }
+    +++++++++++++++++++++++++++++++++++++++
+    pub fn DIFFERENT_FUNCTION_NAME__MUAHAHA()
+        if let Some(ref patch0) = self.patch0 {
+            if let Some(ref patch1) = self.patch1 {
+                return different_patch_files(patch0, patch1)
+            }
+        }
+
+        vec![]
+    }
 }

 fn parse_diff(diff: &str) -> Option<PatchSet> {
diff --git a/src/force_push.rs b/src/force_push.rs
index 33667da..3503c28 100644
--- a/src/force_push.rs
+++ b/src/force_push.rs
@@ -29,10 +29,15 @@ pub fn comment_force_push(diffs: Result<DiffOfDiffs, String>,
                 comment += "TOTALLY THE SAME!";
                 identical_diff = true;
             } else {
-                // TODO: How to expose this diff -- maybe create a secret gist?
-                // But that may raise permissions concerns for users who can read octobot's gists,
-                // but perhaps not the original repo...
                 comment += "Diff changed post-rebase";
+                let different_files = diffs.different_patch_files();
+                if different_files.len() > 0 {
+                    comment += "\n\nChanged files:\n";
+                    for file in different_files {
+                        comment += &format!("* {}", file.path());
+                    }
+                }
+
                 identical_diff = false;
             }
         }, "#;

    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let diffs = Ok(DiffOfDiffs::new(diff0, diff1));

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: abcdef0, after: 1111abc: Diff changed post-rebase\n\n\
        Changed files:\n\
        * src/diffs.rs\n\
        * src/force_push.rs\n\
        ",
        Ok(()));

    force_push::comment_force_push(diffs, vec![], &github, "some-user", "some-repo", &pr,
                                   "abcdef0999999", "1111abc9999999").unwrap();
}
#[test]
fn test_force_push_error() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: abcdef0, after: 1111abc: Unable to calculate diff",
        Ok(()));

    force_push::comment_force_push(Err("Ahh!!".into()), vec![], &github, "some-user", "some-repo", &pr,
                                   "abcdef0999999", "1111abc9999999").unwrap();
}
