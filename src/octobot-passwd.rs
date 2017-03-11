extern crate octobot;
extern crate rpassword;

use ring::rand::SystemRandom;
use rustc_serialize::hex::ToHex;

use octobot::config;
use octobot::server::login;

fn main() {
    if std::env::args().count() < 2 {
        panic!("Usage: octobot-admin <config file>");
    }

    let config_file = std::env::args().nth(1).unwrap();
    let mut config = config::parse(config_file).expect("Error parsing config");

    let pass1 = rpassword::prompt_password_stdout("Enter new password: ").expect("password");
    let pass2 = rpassword::prompt_password_stdout("Retype new password: ").expect("password");

    if pass1 != pass2 {
        println!("Passwords do not match!");
        std::process::exit(1);
    }

   let mut salt_bytes: [u8; 32] = [0; 32];
   SystemRandom::new().fill(&mut bytes).expect("get random");
   let salt = salt_bytes.to_hex();

   let pass_hash = login::hash_password(&pass1, &salt);

   config.admin = Some(config::AdminConfig {
       salt: salt,
       pass_hash: pass_hash,
   });

   // TODO: save it

}
