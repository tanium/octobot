use log::error;
use ring::{digest, pbkdf2};
use rustc_serialize::hex::{FromHex, ToHex};

static DIGEST_ALG: &'static pbkdf2::Algorithm = &pbkdf2::PBKDF2_HMAC_SHA256;
const CREDENTIAL_LEN: usize = digest::SHA256_OUTPUT_LEN;

fn pbdkf2_iterations() -> std::num::NonZeroU32 {
    std::num::NonZeroU32::new(100_000).unwrap()
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

    pass_hash.to_hex()
}

pub fn verify_password(pass: &str, salt: &str, pass_hash: &str) -> bool {
    let pass_hash = match pass_hash.from_hex() {
        Ok(h) => h,
        Err(e) => {
            error!("Invalid password hash stored: {} -- {}", pass_hash, e);
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
        let pw_hash = store_password("the-pass", "some-salt");
        assert_eq!(true, verify_password("the-pass", "some-salt", &pw_hash));
        assert_eq!(false, verify_password("wrong-pass", "some-salt", &pw_hash));
        assert_eq!(false, verify_password("the-pass", "wrong-salt", &pw_hash));
    }
}
