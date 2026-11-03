use log::error;
use ring::rand::{SecureRandom, SystemRandom};
use ring::{digest, pbkdf2};

static DIGEST_ALG: &pbkdf2::Algorithm = &pbkdf2::PBKDF2_HMAC_SHA256;
const CREDENTIAL_LEN: usize = digest::SHA256_OUTPUT_LEN;

fn pbdkf2_iterations() -> std::num::NonZeroU32 {
    std::num::NonZeroU32::new(100_000).unwrap()
}

pub fn new_salt() -> String {
    let mut salt_bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut salt_bytes)
        .expect("get random");
    hex::encode(salt_bytes)
}

pub fn store_password(pass: &str, salt: &str) -> String {
    let mut pass_hash = [0u8; CREDENTIAL_LEN];
    pbkdf2::derive(
        *DIGEST_ALG,
        pbdkf2_iterations(),
        salt.as_bytes(),
        pass.as_bytes(),
        &mut pass_hash,
    );

    hex::encode(pass_hash)
}

pub fn verify_password(pass: &str, salt: &str, pass_hash: &str) -> bool {
    let pass_hash = match hex::decode(pass_hash) {
        Ok(h) => h,
        Err(e) => {
            error!("Invalid password hash stored: {}", e);
            return false;
        }
    };
    pbkdf2::verify(
        *DIGEST_ALG,
        pbdkf2_iterations(),
        salt.as_bytes(),
        pass.as_bytes(),
        &pass_hash,
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password() {
        let salt = new_salt();
        let pw_hash = store_password("the-pass", &salt);
        assert!(verify_password("the-pass", &salt, &pw_hash));
        assert!(!verify_password("wrong-pass", &salt, &pw_hash));
        assert!(!verify_password("the-pass", &new_salt(), &pw_hash));
    }
}
