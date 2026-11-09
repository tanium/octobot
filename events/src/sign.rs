use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use ssh_agent_client_rs::{Client, Identity};
use ssh_key::{Fingerprint, HashAlg, PrivateKey, PublicKey, SshSig};

use crate::NAMESPACE;

/// Signs subscribe challenges with one of the user's keys on GitHub.
pub trait Signer: Send + Sync {
    /// Signs with a key whose SHA-256 fingerprint is in `registered`, or fails if it holds none.
    fn sign(&self, msg: &[u8], registered: &[Fingerprint]) -> Result<SshSig>;
}

/// Signs through ssh-agent, so hardware and password-manager keys work without exposing them.
pub struct AgentSigner {
    socket: PathBuf,
    key: Option<PublicKey>,
}

impl AgentSigner {
    /// Uses the agent at `SSH_AUTH_SOCK`.
    pub fn from_env() -> Result<AgentSigner> {
        let socket = std::env::var_os("SSH_AUTH_SOCK").context("SSH_AUTH_SOCK is not set")?;
        Ok(AgentSigner {
            socket: socket.into(),
            key: None,
        })
    }

    /// Signs with `key`. Without one, the agent's first key registered on GitHub signs.
    pub fn with_key(self, key: PublicKey) -> AgentSigner {
        AgentSigner {
            key: Some(key),
            ..self
        }
    }
}

impl Signer for AgentSigner {
    fn sign(&self, msg: &[u8], registered: &[Fingerprint]) -> Result<SshSig> {
        let mut agent = Client::connect(&self.socket)
            .with_context(|| format!("connecting to ssh-agent at {}", self.socket.display()))?;
        let key = match &self.key {
            Some(key) => {
                if !is_registered(key, registered) {
                    bail!("key {} is not registered on GitHub", fingerprint(key));
                }
                key.clone()
            }
            None => {
                let keys = agent
                    .list_all_identities()?
                    .into_iter()
                    .filter_map(|identity| match identity {
                        Identity::PublicKey(key) => Some(key.into_owned()),
                        Identity::Certificate(_) => None,
                    });
                first_registered(keys, registered)
                    .ok_or_else(|| anyhow!("none of ssh-agent's keys are registered on GitHub"))?
            }
        };

        let data = SshSig::signed_data(NAMESPACE, HashAlg::Sha512, msg)?;
        let signature = agent.sign(key.clone(), &data)?;
        Ok(SshSig::new(
            key.key_data().clone(),
            NAMESPACE,
            HashAlg::Sha512,
            signature,
        )?)
    }
}

impl Signer for PrivateKey {
    fn sign(&self, msg: &[u8], registered: &[Fingerprint]) -> Result<SshSig> {
        if !is_registered(self.public_key(), registered) {
            bail!(
                "key {} is not registered on GitHub",
                fingerprint(self.public_key())
            );
        }
        Ok(PrivateKey::sign(self, NAMESPACE, HashAlg::Sha512, msg)?)
    }
}

fn first_registered(
    mut keys: impl Iterator<Item = PublicKey>,
    registered: &[Fingerprint],
) -> Option<PublicKey> {
    keys.find(|key| is_registered(key, registered))
}

fn is_registered(key: &PublicKey, registered: &[Fingerprint]) -> bool {
    registered.contains(&fingerprint(key))
}

fn fingerprint(key: &PublicKey) -> Fingerprint {
    key.fingerprint(HashAlg::Sha256)
}

#[cfg(test)]
mod tests {
    use ssh_key::Algorithm;
    use ssh_key::rand_core::OsRng;

    use super::*;

    fn new_key() -> PrivateKey {
        PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap()
    }

    #[test]
    fn test_first_registered_skips_unregistered_keys() {
        let (other, wanted) = (new_key(), new_key());
        let keys = vec![other.public_key().clone(), wanted.public_key().clone()];

        let picked = first_registered(keys.into_iter(), &[fingerprint(wanted.public_key())]);
        assert_eq!(Some(wanted.public_key().clone()), picked);
    }

    #[test]
    fn test_first_registered_none_when_no_key_matches() {
        let keys = vec![new_key().public_key().clone()];

        assert_eq!(None, first_registered(keys.into_iter(), &[]));
    }

    #[test]
    fn test_private_key_refuses_to_sign_unregistered() {
        let key = new_key();

        assert!(Signer::sign(&key, b"msg", &[]).is_err());
        assert!(Signer::sign(&key, b"msg", &[fingerprint(key.public_key())]).is_ok());
    }
}
