use ring::rand::SecureRandom;
use ring::rand::SystemRandom;

use octobot_lib::config;
use octobot_lib::passwd;

fn main() {
    if std::env::args().count() < 3 {
        panic!("Usage: octobot-admin <config file> <admin username>");
    }

    let config_file = std::env::args().nth(1).unwrap();
    let admin_name = std::env::args().nth(2).unwrap();

    let mut config = config::new(config_file.clone().into()).expect("Error parsing config");

    let pass1 = rpassword::prompt_password("Enter new password: ").expect("password");
    let pass2 = rpassword::prompt_password("Retype new password: ").expect("password");

    if pass1 != pass2 {
        println!("Passwords do not match!");
        std::process::exit(1);
    }

    let mut salt_bytes: [u8; 32] = [0; 32];
    SystemRandom::new()
        .fill(&mut salt_bytes)
        .expect("get random");
    let salt: String = hex::encode(salt_bytes);

    let pass_hash = passwd::store_password(&pass1, &salt);

    if admin_name == "--metrics" {
        config.metrics = Some(config::MetricsConfig { salt, pass_hash });
    } else {
        config.admin = Some(config::AdminConfig {
            name: admin_name,
            salt,
            pass_hash,
        });
    }

    config.save(&config_file).expect("save config file");

    println!("Successfully changed password!");
}
