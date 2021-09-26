use std::io::{self, Write};

use octobot_lib::config;

// A test utility to connect to LDAP to verify the configuration
fn main() {
    if std::env::args().count() < 3 {
        panic!("Usage: ldap-check <config file> <command: auth | search>");
    }

    let config_file = std::env::args().nth(1).unwrap();
    let command = std::env::args().nth(2).unwrap();

    let config = config::new(config_file.into()).expect("Error parsing config");
    let ldap_config = config.ldap.expect("No LDAP config");

    if command != "auth" && command != "search" {
        panic!("Invalid command: {}. Must specify auth or search", command);
    }

    if command == "auth" {
        let user = read_input("Enter username: ");
        let pass = rpassword::prompt_password_stdout("Enter password: ").expect("password");

        match octobot_ldap::auth(&user, &pass, &ldap_config) {
            Ok(true) => println!("Successfully authenticated"),
            Ok(false) => println!("Failed authentication"),
            Err(e) => println!("Failed authentication: {}", e),
        }
    } else if command == "search" {

        let max = 1000;
        match octobot_ldap::search(&ldap_config, None, max) {
            Err(e) => println!("Failed search: {}", e),
            Ok(results) => {
                println!("Found {} results (max {})", results.len(), max);
                for res in results {
                    println!(" - {}", res.dn);
                    //println!("   {:?}", res.attrs);
                }
            }
        }
    }
}

fn read_input(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().unwrap_or(());

    let mut line = String::new();
    io::stdin().read_line(&mut line).expect("input");
    line.trim().to_string()
}
