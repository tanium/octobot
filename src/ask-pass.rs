extern crate regex;

use regex::Regex;

fn main() {
    let octobot_pass = match std::env::var("OCTOBOT_PASS") {
        Ok(val) => val,
        Err(e) => panic!("Could not read OCTOBOT_PASS value: {}", e),
    };

    let octobot_host = match std::env::var("OCTOBOT_HOST") {
        Ok(val) => val,
        Err(e) => panic!("Could not read OCTOBOT_HOST value: {}", e),
    };

    let prompt = std::env::args().nth(1).unwrap_or(String::new());
    let regex = Regex::new(r"Password for '.*@(.*)'").unwrap();

    let host = match regex.captures(&prompt) {
        Some(c) => c[1].to_string(),
        None => String::new(),
    };

    // only care about a single host for now, but keep this logic just incase...
    if host != octobot_host {
        println!("this is the wrong password");

    } else {
        println!("{}", octobot_pass);
    }
}
