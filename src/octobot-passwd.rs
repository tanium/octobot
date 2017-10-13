extern crate octobot;
extern crate ring;
extern crate rpassword;
extern crate rustc_serialize;

use ring::rand::SecureRandom;
use ring::rand::SystemRandom;
use rustc_serialize::hex::ToHex;

use octobot::config;
use octobot::server::login;

fn main() {
    if std::env::args().count() < 3 {
        panic!("Usage: octobot-admin <config file> <admin username>");
    }

    let config_file = std::env::args().nth(1).unwrap();
    let admin_name = std::env::args().nth(2).unwrap();

    let mut config = config::parse(&config_file).expect("Error parsing config");

    let pass1 = rpassword::prompt_password_stdout("Enter new password: ").expect("password");
    let pass2 = rpassword::prompt_password_stdout("Retype new password: ").expect("password");

    if pass1 != pass2 {
        println!("Passwords do not match!");
        std::process::exit(1);
    }

   let mut salt_bytes: [u8; 32] = [0; 32];
   SystemRandom::new().fill(&mut salt_bytes).expect("get random");
   let salt: String = salt_bytes.to_hex().to_string();

   let pass_hash = login::store_password(&pass1, &salt);

   config.admin = Some(config::AdminConfig {
       name: admin_name,
       salt: salt,
       pass_hash: pass_hash,
   });

   config.save(&config_file).expect("save config file");

   println!("Successfully changed password!");
}
