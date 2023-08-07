mod mocks;

use anyhow::anyhow;

use mocks::mock_github::MockGithub;

use octobot_lib::github;
use octobot_ops::diffs::DiffOfDiffs;
use octobot_ops::force_push;

#[tokio::test]
async fn test_force_push_identical() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;
    pr.head.ref_name = "the-pr-branch".into();

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok(DiffOfDiffs::new(&diff, &diff));

    let github = MockGithub::new();
    github.mock_comment_pull_request(
        "some-user",
        "some-repo",
        32,
        "Force-push detected: before: abcdef0, after: 1111abc: Identical diff post-rebase.",
        Ok(()),
    );

    github.mock_get_timeline("some-user", "some-repo", 32, Ok(vec![]));

    force_push::comment_force_push(
        diffs,
        &github,
        "some-user",
        "some-repo",
        &pr,
        "abcdef0999999",
        "1111abc9999999",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_force_push_different() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let diffs = Ok(DiffOfDiffs::new("diff1", "diff2"));

    let github = MockGithub::new();
    github.mock_comment_pull_request(
        "some-user",
        "some-repo",
        32,
        "Force-push detected: before: abcdef0, after: 1111abc: Diff changed post-rebase.",
        Ok(()),
    );

    force_push::comment_force_push(
        diffs,
        &github,
        "some-user",
        "some-repo",
        &pr,
        "abcdef0999999",
        "1111abc9999999",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_force_push_different_with_details() {
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
    github.mock_comment_pull_request(
        "some-user",
        "some-repo",
        32,
        "Force-push detected: before: abcdef0, after: 1111abc: Diff changed post-rebase.\n\n\
        Changed files:\n\
        * src/diffs.rs\n\
        * src/force_push.rs\n\
        ",
        Ok(()),
    );

    force_push::comment_force_push(
        diffs,
        &github,
        "some-user",
        "some-repo",
        &pr,
        "abcdef0999999",
        "1111abc9999999",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_force_push_error() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let github = MockGithub::new();
    github.mock_comment_pull_request(
        "some-user",
        "some-repo",
        32,
        "Force-push detected: before: abcdef0, after: 1111abc: Unable to calculate diff.",
        Ok(()),
    );

    force_push::comment_force_push(
        Err(anyhow!("Ahh!!")),
        &github,
        "some-user",
        "some-repo",
        &pr,
        "abcdef0999999",
        "1111abc9999999",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_force_push_identical_no_previous_approve_dismissal() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;
    pr.head.ref_name = "the-pr-branch".into();

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok(DiffOfDiffs::new(&diff, &diff));

    let github = MockGithub::new();
    github.mock_comment_pull_request(
        "some-user",
        "some-repo",
        32,
        "Force-push detected: before: abcdef0, after: 1111abc: Identical diff post-rebase.",
        Ok(()),
    );

    github.mock_get_timeline("some-user", "some-repo", 32, Ok(vec![]));

    // Do not mock approve_pull_request: Should not re-approve

    force_push::comment_force_push(
        diffs,
        &github,
        "some-user",
        "some-repo",
        &pr,
        "abcdef0999999",
        "1111abc9999999",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_force_push_identical_no_previous_approval() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;
    pr.head.ref_name = "the-pr-branch".into();

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok(DiffOfDiffs::new(&diff, &diff));

    let github = MockGithub::new();
    github.mock_comment_pull_request(
        "some-user",
        "some-repo",
        32,
        "Force-push detected: before: abcdef0, after: 1111abc: Identical diff post-rebase.",
        Ok(()),
    );

    // Claims to have dismissed a review, but no such review is in the timeline. Skip.
    github.mock_get_timeline(
        "some-user",
        "some-repo",
        32,
        Ok(vec![github::TimelineEvent::new_dismissed_review(
            github::DismissedReview::by_commit("approved", "abcdef0999999", 1234),
        )]),
    );

    // Do not mock approve_pull_request: Should not re-approve

    force_push::comment_force_push(
        diffs,
        &github,
        "some-user",
        "some-repo",
        &pr,
        "abcdef0999999",
        "1111abc9999999",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_force_push_identical_reapprove() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;
    pr.head.ref_name = "the-pr-branch".into();

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok(DiffOfDiffs::new(&diff, &diff));

    let github = MockGithub::new();

    let before_hash = "abcdef0999999";
    let after_hash = "1111abc9999999";

    // This timeline is valid for reapproval because the latest dismissal came from a code change
    // with a commit hash that is the exact same as the `before_hash` for this force push.
    github.mock_get_timeline(
        "some-user",
        "some-repo",
        32,
        Ok(vec![
            github::TimelineEvent::new_dismissed_review(github::DismissedReview::by_user(
                "approved",
                "I don't like this",
            )),
            github::TimelineEvent::new("some"),
            github::TimelineEvent::new("other"),
            github::TimelineEvent::new("event"),
            github::TimelineEvent::new_review(
                before_hash,
                1234,
                github::User::new("joe-reviewer"),
                "http://the-review-url",
            ),
            github::TimelineEvent::new_dismissed_review(github::DismissedReview::by_commit(
                "approved", after_hash, 1234,
            )),
        ]),
    );

    github.mock_approve_pull_request(
        "some-user",
        "some-repo",
        32,
        after_hash,
        Some(
            "Force-push detected: before: abcdef0, after: 1111abc: Identical diff post-rebase.\n\n\
             Reapproved based on review by [joe-reviewer](http://the-review-url)",
        ),
        Ok(()),
    );

    force_push::comment_force_push(
        diffs,
        &github,
        "some-user",
        "some-repo",
        &pr,
        before_hash,
        after_hash,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_force_push_identical_wrong_previous_approval() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;
    pr.head.ref_name = "the-pr-branch".into();

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok(DiffOfDiffs::new(&diff, &diff));

    let github = MockGithub::new();
    github.mock_comment_pull_request(
        "some-user",
        "some-repo",
        32,
        "Force-push detected: before: abcdef0, after: 1111abc: Identical diff post-rebase.",
        Ok(()),
    );

    let before_hash = "abcdef0999999";
    let after_hash = "1111abc9999999";

    github.mock_get_timeline(
        "some-user",
        "some-repo",
        32,
        Ok(vec![
            github::TimelineEvent::new_review(
                before_hash,
                1234,
                github::User::new("joe-reviewer"),
                "http://the-review-url",
            ),
            github::TimelineEvent::new_dismissed_review(github::DismissedReview::by_commit(
                "approved", after_hash, 1234,
            )),
            // The latest dismissal is *not* by commit id, so don't reapprove
            github::TimelineEvent::new_dismissed_review(github::DismissedReview::by_user(
                "approved",
                "I don't like this",
            )),
        ]),
    );

    // Do not mock approve_pull_request: Should not re-approve

    force_push::comment_force_push(
        diffs,
        &github,
        "some-user",
        "some-repo",
        &pr,
        before_hash,
        after_hash,
    )
    .await
    .unwrap();
}
