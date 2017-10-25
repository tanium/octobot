use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Git {
    pub host: String,
    pub token: String,
    repo_dir: PathBuf,
}

impl Git {
    pub fn new(host: &str, token: &str, repo_dir: &Path) -> Git {
        Git {
            host: host.to_string(),
            token: token.to_string(),
            repo_dir: repo_dir.to_owned(),
        }
    }

    pub fn run(&self, args: &[&str]) -> Result<String, String> {
        self.do_run(args, None)
    }

    pub fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Result<String, String> {
        self.do_run(args, Some(stdin))
    }

    fn ask_pass_path(&self) -> String {
        let ask_pass = "octobot-ask-pass";
        match env::current_exe() {
            Ok(ref exe) if exe.parent().is_some() => {
                exe.parent().unwrap().join(ask_pass).to_string_lossy().into_owned()
            }
            _ => ask_pass.to_string(),
        }
    }

    pub fn has_branch(&self, branch: &str) -> Result<bool, String> {
        let output = self.run(&["branch"])?;
        Ok(Git::branches_output_contains(&output, branch))
    }

    pub fn current_branch(&self) -> Result<String, String> {
        self.run(&["rev-parse", "--abbrev-ref", "HEAD"])
    }

    pub fn does_branch_contain(&self, git_ref: &str, branch: &str) -> Result<bool, String> {
        let output = self.run(&["branch", "--contains", git_ref])?;
        Ok(Git::branches_output_contains(&output, branch))
    }

    fn branches_output_contains(output: &str, branch: &str) -> bool {
        // Output is trimmed, so first entry (if not current branch) won't have an asterisk.
        // Otherwise, skip two characters to account for alignment w/ asterisk.
        output.lines().any(|b| b == branch || b.len() > 2 && branch == &b[2..])
    }

    // Find the commit at which |leaf_ref| forked from |base_branch|.
    // This can find which commits belong to a PR.
    // Returns the ref found in the base branch that this git_ref came from.
    pub fn find_base_branch_commit(&self, leaf_ref: &str, base_branch: &str) -> Result<String, String> {
        match self.run(&["merge-base", "--fork-point", base_branch, leaf_ref]) {
            Ok(base) => Ok(base),
            Err(_) => self.run(&["merge-base", base_branch, leaf_ref]),
        }
    }

    pub fn clean(&self) -> Result<(), String> {
        self.run(&["reset", "--hard"])?;
        self.run(&["clean", "-fdx"])?;
        Ok(())
    }

    // checking a branch named |new_branch_name| and ensure it is up to date with |source_ref|
    // |source_ref| can be a commit hash or an origin/branch-name.
    pub fn checkout_branch(&self, new_branch_name: &str, source_ref: &str) -> Result<(), String> {
        self.run(&["checkout", "-B", new_branch_name, source_ref])?;
        Ok(())
    }

    pub fn diff(&self, base: &str, head: &str) -> Result<String, String> {
        self.run(&["diff", base, head, "-w"])
    }

    // returns (title, body)
    pub fn get_commit_desc(&self, commit_hash: &str) -> Result<(String, String), String> {
        let message = self.run(&["log", "-1", "--pretty=%B", commit_hash])?;

        let mut lines = message.lines();
        let title: String = lines.next().unwrap_or("").into();
        let body: Vec<&str> = lines.skip_while(|ref l| l.trim().len() == 0).collect();

        Ok((title, body.join("\n")))
    }

    fn do_run(&self, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
        debug!("Running git with args: {:?}", args);
        let cmd = Command::new("git")
            .current_dir(&self.repo_dir)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .args(args)
            .env("GIT_ASKPASS", &self.ask_pass_path())
            .env("OCTOBOT_HOST", &self.host)
            .env("OCTOBOT_PASS", &self.token)
            .spawn();

        let mut child = match cmd {
            Ok(c) => c,
            Err(e) => return Err(format!("Error starting git (args: {:?}): {}", args, e)),
        };

        if let Some(ref stdin) = stdin {
            if let Some(ref mut child_stdin) = child.stdin {
                if let Err(e) = child_stdin.write_all(stdin.as_bytes()) {
                    return Err(format!("Error writing to stdin: {}", e));
                }
            }
        }

        let result = match child.wait_with_output() {
            Ok(r) => r,
            Err(e) => return Err(format!("Error running git (args: {:?}): {}", args, e)),
        };

        let mut output = String::new();
        if result.stdout.len() > 0 {
            output += String::from_utf8_lossy(&result.stdout).as_ref();
        }

        if !result.status.success() {
            if result.stderr.len() > 0 {
                output += String::from_utf8_lossy(&result.stderr).as_ref();
            }
            Err(format!(
                "Error running git (exit code {}, args: {:?}):\n{}",
                result.status.code().unwrap_or(-1),
                args,
                output
            ))
        } else {

            Ok(output.trim().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_branches_output_contains() {
        assert!(Git::branches_output_contains("test", "test"));
        assert!(Git::branches_output_contains("* test", "test"));
        assert!(!Git::branches_output_contains("* tests", "test"));

        assert!(Git::branches_output_contains("test\ntwo", "two"));
        assert!(Git::branches_output_contains("test\n* two", "two"));
        assert!(!Git::branches_output_contains("test\n* twos", "two"));
    }
}
