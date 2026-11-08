use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use ssh_agent_client_rs::{Client, Identity};
use ssh_key::{HashAlg, PrivateKey, PublicKey, SshSig};

use crate::NAMESPACE;

/// Signs subscribe challenges with a key registered to the user on GitHub.
pub trait Signer: Send + Sync {
    fn sign(&self, msg: &[u8]) -> Result<SshSig>;
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

    /// Signs with `key`. Without one, the agent's first identity signs, which fails when that
    /// key is not registered on GitHub.
    pub fn with_key(self, key: PublicKey) -> AgentSigner {
        AgentSigner {
            key: Some(key),
            ..self
        }
    }
}

impl Signer for AgentSigner {
    fn sign(&self, msg: &[u8]) -> Result<SshSig> {
        let mut agent = Client::connect(&self.socket)
            .with_context(|| format!("connecting to ssh-agent at {}", self.socket.display()))?;
        let key = match &self.key {
            Some(key) => key.clone(),
            None => agent
                .list_all_identities()?
                .into_iter()
                .find_map(|identity| match identity {
                    Identity::PublicKey(key) => Some(key.into_owned()),
                    Identity::Certificate(_) => None,
                })
                .ok_or_else(|| anyhow!("ssh-agent has no keys"))?,
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
    fn sign(&self, msg: &[u8]) -> Result<SshSig> {
        Ok(PrivateKey::sign(self, NAMESPACE, HashAlg::Sha512, msg)?)
    }
}
