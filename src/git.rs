use std::env;
use std::io::Write;
use std::process::{Command, Stdio};
use std::path::{Path, PathBuf};

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
        let output = try!(self.run(&["branch"]));
        // skip first two characters for the bullet point
        Ok(output.lines().any(|b| b.len() > 2 && branch == &b[2..]))
    }

    pub fn current_branch(&self) -> Result<String, String> {
        self.run(&["rev-parse", "--abbrev-ref", "HEAD"])
    }

    pub fn does_branch_contain(&self, git_ref: &str, branch: &str) -> Result<bool, String> {
        let output = try!(self.run(&["branch", "--contains", git_ref]));
        // return branches, stripping out the bullet point
        Ok(output.lines().any(|b| b.len() > 2 && branch == &b[2..]))
    }

    // Find the commit at which |leaf_ref| forked from |base_branch|.
    // This can find which commits belong to a PR.
    // Returns the ref found in the base branch that this git_ref came from.
    pub fn find_base_branch_commit(&self, leaf_ref: &str, base_branch: &str) -> Result<String, String> {
        self.run(&["merge-base", "--fork-point", base_branch, leaf_ref])
    }

    pub fn clean(&self) -> Result<(), String> {
        try!(self.run(&["reset", "--hard"]));
        try!(self.run(&["clean", "-fdx"]));
        Ok(())
    }

    // checking a branch named |new_branch_name| and ensure it is up to date with |source_branch|
    // |source_branch| can be a commit hash or an origin/branch-name.
    pub fn checkout_branch(&self, new_branch_name: &str, source_branch: &str) ->  Result<(), String> {
        let current_branch = try!(self.current_branch());
        if current_branch == new_branch_name {
            try!(self.run(&["reset", "--hard", &source_branch]));
        } else {
            // delete if it exists
            let has_branch = try!(self.has_branch(new_branch_name));
            if has_branch {
                try!(self.run(&["branch", "-D", new_branch_name]));
            }
            // recreate branch
            try!(self.run(&["checkout", "-b", new_branch_name, &source_branch]));
        }

        Ok(())
    }

    pub fn diff(&self, base: &str, head: &str) -> Result<String, String> {
        self.run(&["diff", base, head, "-w"])
    }

    // returns (title, body)
    pub fn get_commit_desc(&self, commit_hash: &str) -> Result<(String, String), String> {
        let lines: Vec<String> = try!(self.run(&["log", "-1", "--pretty=%B", commit_hash]))
            .split("\n")
            .map(|l| l.trim().to_string())
            .collect();

        if lines.len() == 0 {
            return Err(format!("Empty commit message found!"));
        }

        let title = lines[0].clone();

        let mut body = String::new();
        // skip the blank line
        if lines.len() > 2 {
            body = lines[2..].join("\n");
            body += "\n";
        }

        Ok((title, body))
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
            Err(e) => return Err(format!("Error starting git: {}", e)),
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
            Err(e) => return Err(format!("Error running git: {}", e)),
        };

        let mut output = String::new();
        if result.stdout.len() > 0 {
            output += String::from_utf8_lossy(&result.stdout).as_ref();
        }

        if !result.status.success() {
            if result.stderr.len() > 0 {
                output += String::from_utf8_lossy(&result.stderr).as_ref();
            }
            Err(format!("Error running git (exit code {}):\n{}",
                        result.status.code().unwrap_or(-1),
                        output))
        } else {

            Ok(output.trim().to_string())
        }
    }
}
