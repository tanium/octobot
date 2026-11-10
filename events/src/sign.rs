use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use ssh_agent_client_rs::{Client, Identity};
use ssh_key::{HashAlg, PrivateKey, PublicKey, SshSig};

use crate::NAMESPACE;

/// Signs subscribe challenges, with the first of its keys that the user has on GitHub.
pub trait Signer: Send + Sync {
    /// The keys it can sign with, in order of preference.
    fn keys(&self) -> Result<Vec<PublicKey>>;
    fn sign(&self, key: &PublicKey, msg: &[u8]) -> Result<SshSig>;
}

/// Signs through ssh-agent, so hardware and password-manager keys work without exposing them.
pub struct AgentSigner {
    socket: PathBuf,
    key: Option<PublicKey>,
}

impl AgentSigner {
    /// Uses the ssh-agent listening on `socket`: OpenSSH's, gpg-agent's, 1Password's,
    /// Secretive's, or any other.
    pub fn new(socket: impl Into<PathBuf>) -> AgentSigner {
        AgentSigner {
            socket: socket.into(),
            key: None,
        }
    }

    /// Uses the agent at `SSH_AUTH_SOCK`.
    pub fn from_env() -> Result<AgentSigner> {
        let socket = std::env::var_os("SSH_AUTH_SOCK").context("SSH_AUTH_SOCK is not set")?;
        Ok(AgentSigner::new(socket))
    }

    /// Signs only with `key`, rather than the agent's first key on GitHub.
    pub fn with_key(self, key: PublicKey) -> AgentSigner {
        AgentSigner {
            key: Some(key),
            ..self
        }
    }

    fn connect(&self) -> Result<Client> {
        Client::connect(&self.socket)
            .with_context(|| format!("connecting to ssh-agent at {}", self.socket.display()))
    }
}

impl Signer for AgentSigner {
    fn keys(&self) -> Result<Vec<PublicKey>> {
        if let Some(key) = &self.key {
            return Ok(vec![key.clone()]);
        }
        Ok(self
            .connect()?
            .list_all_identities()?
            .into_iter()
            .filter_map(|identity| match identity {
                Identity::PublicKey(key) => Some(key.into_owned()),
                Identity::Certificate(_) => None,
            })
            .collect())
    }

    fn sign(&self, key: &PublicKey, msg: &[u8]) -> Result<SshSig> {
        let data = SshSig::signed_data(NAMESPACE, HashAlg::Sha512, msg)?;
        let signature = self.connect()?.sign(key.clone(), &data)?;
        Ok(SshSig::new(
            key.key_data().clone(),
            NAMESPACE,
            HashAlg::Sha512,
            signature,
        )?)
    }
}

impl Signer for PrivateKey {
    fn keys(&self) -> Result<Vec<PublicKey>> {
        Ok(vec![self.public_key().clone()])
    }

    fn sign(&self, key: &PublicKey, msg: &[u8]) -> Result<SshSig> {
        if key.key_data() != self.public_key().key_data() {
            bail!("not this signer's key");
        }
        Ok(PrivateKey::sign(self, NAMESPACE, HashAlg::Sha512, msg)?)
    }
}

#[cfg(test)]
mod tests {
    use ssh_key::Algorithm;
    use ssh_key::rand_core::OsRng;

    use super::*;

    #[test]
    fn test_agent_signer_uses_the_given_socket() {
        let error = AgentSigner::new("/nonexistent/agent.sock")
            .keys()
            .unwrap_err();
        assert!(format!("{:#}", error).contains("/nonexistent/agent.sock"));
    }

    #[test]
    fn test_private_key_signs_only_with_itself() {
        let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        let other = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();

        assert!(Signer::sign(&key, key.public_key(), b"msg").is_ok());
        assert!(Signer::sign(&key, other.public_key(), b"msg").is_err());
    }
}
