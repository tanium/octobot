use std::env;
use std::io::Write;
use std::process::{Command, Stdio};
use std::path::Path;

pub struct Git {
    pub host: String,
    pub token: String,
}

impl Git {
    pub fn new(host: &str, token: &str) -> Git {
        Git {
            host: host.to_string(),
            token: token.to_string(),
        }
    }

    pub fn run(&self, args: &[&str], cwd: &Path) -> Result<String, String> {
        self.do_run(args, cwd, None)
    }

    pub fn run_with_stdin(&self, args: &[&str], cwd: &Path, stdin: &str) -> Result<String, String> {
        self.do_run(args, cwd, Some(stdin))
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

    pub fn has_branch(&self, branch: &str, cwd: &Path) -> Result<bool, String> {
        let output = try!(self.run(&["branch"], cwd));
        // skip first two characters for the bullet point
        Ok(output.lines().any(|b| b.len() > 2 && branch == &b[2..]))
    }

    pub fn current_branch(&self, cwd: &Path) -> Result<String, String> {
        self.run(&["rev-parse", "--abbrev-ref", "HEAD"], cwd)
    }

    pub fn clean(&self, cwd: &Path) -> Result<(), String> {
        try!(self.run(&["reset", "--hard"], cwd));
        try!(self.run(&["clean", "-fdx"], cwd));
        Ok(())
    }

    fn do_run(&self, args: &[&str], cwd: &Path, stdin: Option<&str>) -> Result<String, String> {
        debug!("Running git with args: {:?}", args);
        let cmd = Command::new("git")
            .current_dir(cwd)
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
