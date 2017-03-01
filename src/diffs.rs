use unidiff::{PatchSet, PatchedFile, Hunk, Line};

#[derive(Debug)]
pub struct DiffOfDiffs {
    diff0: String,
    diff1: String,
    patch0: Option<PatchSet>,
    patch1: Option<PatchSet>,
}

impl DiffOfDiffs {
    pub fn new(diff0: &str, diff1: &str) -> DiffOfDiffs {
        DiffOfDiffs {
            diff0: diff0.into(),
            diff1: diff1.into(),
            patch0: parse_diff(diff0),
            patch1: parse_diff(diff1),
        }
    }

    pub fn are_equal(&self) -> bool {
        // try to parse diffs and if can't parse either,
        // then default to straight string comparison
        let patch0 = match self.patch0 {
            None => return self.diff0 == self.diff1,
            Some(ref p) => p,
        };
        let patch1 = match self.patch1 {
            None => return self.diff0 == self.diff1,
            Some(ref p) => p,
        };

        if patch0.is_empty() || patch1.is_empty() {
            return self.diff0 == self.diff1;
        }

        are_patch_sets_equal(patch0, patch1)
    }
}

fn parse_diff(diff: &str) -> Option<PatchSet> {
    let mut patch = PatchSet::new();
    match patch.parse(diff) {
        Ok(_) => Some(patch),
        Err(e) => {
            debug!("Unable to parse patch set: {}\n---\n{}\n---\n", e, diff);
            None
        }
    }
}


fn are_patch_sets_equal(patch0: &PatchSet, patch1: &PatchSet) -> bool {
    if patch0.len() != patch1.len() {
        return false;
    }

    for i in 0..patch0.len() {
        if !are_patched_files_equal(&patch0[i], &patch1[i]) {
            return false;
        }
    }

    true
}

fn are_patched_files_equal(file0: &PatchedFile, file1: &PatchedFile) -> bool {
    if file0.len() != file1.len() {
        return false;
    }

    for hunk_num in 0..file0.len() {
        if !are_patch_hunks_equal(&file0[hunk_num], &file1[hunk_num]) {
            return false;
        }
    }

    true
}

fn are_patch_hunks_equal(hunk0: &Hunk, hunk1: &Hunk) -> bool {
    if hunk0.len() != hunk1.len() {
        return false;
    }

    for line_num in 0..hunk0.len() {
        if !are_patch_lines_equal(&hunk0[line_num], &hunk1[line_num]) {
            return false;
        }
    }

    true
}

fn are_patch_lines_equal(line0: &Line, line1: &Line) -> bool {
    // only check for line type and line value for equality,
    // ignore source/target/diff line numbers
    line0.line_type == line1.line_type && line0.value == line1.value
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_diff_contents() {
        assert_eq!(true, DiffOfDiffs::new("some-non-diff", "some-non-diff").are_equal());
        assert_eq!(false, DiffOfDiffs::new("some-non-diff", "some-other-non-diff").are_equal());
    }

    #[test]
    fn test_git_diff_and_diff_line_numbers() {
        let diff0 = r#"
diff --git a/Cargo.toml b/Cargo.toml
index 43f2e75..42eaec0 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -31,6 +31,7 @@ thread-id = "3.0.0"
 threadpool = "1.3.2"
 time = "0.1.35"
 toml = "0.2.1"
+unidiff = "0.2.0"
 url = "1.2.4"

 [lib]
diff --git a/src/lib.rs b/src/lib.rs
index 4442a2c..007a2cf 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -11,6 +11,7 @@ extern crate serde;
 extern crate serde_json;
 extern crate threadpool;
 extern crate toml;
+extern crate unidiff;
 extern crate url;

 #[macro_use]
@@ -19,6 +20,7 @@ extern crate log;
 extern crate serde_derive;

 pub mod config;
+pub mod diffs;
 pub mod dir_pool;
 pub mod force_push;
 pub mod git;"#;

        assert_eq!(true, DiffOfDiffs::new(&diff0, &diff0).are_equal());

        // same diff, but diff line numbers
        let diff1 = r#"
diff --git a/Cargo.toml b/Cargo.toml
index 43f2e75..42eaec0 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -41,6 +41,7 @@ thread-id = "3.0.0"
 threadpool = "1.3.2"
 time = "0.1.35"
 toml = "0.2.1"
+unidiff = "0.2.0"
 url = "1.2.4"

 [lib]
diff --git a/src/lib.rs b/src/lib.rs
index 4442a2c..007a2cf 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,6 +1,7 @@ extern crate serde;
 extern crate serde_json;
 extern crate threadpool;
 extern crate toml;
+extern crate unidiff;
 extern crate url;

 #[macro_use]
@@ -119,6 +120,7 @@ extern crate log;
 extern crate serde_derive;

 pub mod config;
+pub mod diffs;
 pub mod dir_pool;
 pub mod force_push;
 pub mod git;"#;

        assert_eq!(true, DiffOfDiffs::new(&diff0, &diff1).are_equal());
    }

    #[test]
    fn test_different_diff() {
        let diff0 = r#"
diff --git a/Cargo.toml b/Cargo.toml
index 43f2e75..42eaec0 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -31,6 +31,7 @@ thread-id = "3.0.0"
 threadpool = "1.3.2"
 time = "0.1.35"
 toml = "0.2.1"
+unidiff = "0.2.0"
 url = "1.2.4"

 [lib]
diff --git a/src/lib.rs b/src/lib.rs
index 4442a2c..007a2cf 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -11,6 +11,7 @@ extern crate serde;
 extern crate serde_json;
 extern crate threadpool;
 extern crate toml;
+extern crate unidiff;
 extern crate url;

 #[macro_use]
@@ -19,6 +20,7 @@ extern crate log;
 extern crate serde_derive;

 pub mod config;
+pub mod diffs;
 pub mod dir_pool;
 pub mod force_push;
 pub mod git;"#;

        let diff1 = r#"
diff --git a/Cargo.toml b/Cargo.toml
index 43f2e75..42eaec0 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -41,6 +41,7 @@ thread-id = "3.0.0"
 threadpool = "1.3.2"
 time = "0.1.35"
 toml = "0.2.1"
+unidiff = "0.2.0"
 url = "1.2.4"

 [lib]
diff --git a/src/lib.rs b/src/lib.rs
index 4442a2c..007a2cf 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,6 +1,7 @@ extern crate serde;
 extern crate serde_json;
 extern crate threadpool;
 extern crate toml;
+extern crate unidiff;
 extern crate url;

 #[macro_use]
@@ -119,6 +120,7 @@ extern crate log;
 extern crate serde_derive;

 pub mod ** OTHER_THING_HERE **;
+pub mod diffs;
 pub mod dir_pool;
 pub mod force_push;
 pub mod git;"#;

        assert_eq!(false, DiffOfDiffs::new(&diff0, &diff1).are_equal());
    }
}
