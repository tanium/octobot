use std::fmt;

use anyhow::bail;

use crate::errors::*;

/// A GitHub login that is safe to put in an API path: lowercase (GitHub logins are
/// case-insensitive) ASCII letters, digits, `-` and `_`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Login(String);

impl Login {
    pub fn parse(login: &str) -> Result<Login> {
        let login = login.to_lowercase();
        if login.is_empty()
            || !login
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("invalid GitHub login: {:?}", login);
        }
        Ok(Login(login))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lowercases() {
        assert_eq!(
            "matt-hauck_2",
            Login::parse("Matt-Hauck_2").unwrap().as_str()
        );
    }

    #[test]
    fn test_parse_rejects_path_characters() {
        assert!(Login::parse("../orgs/x").is_err());
        assert!(Login::parse("joe?x=1").is_err());
        assert!(Login::parse("").is_err());
    }
}
