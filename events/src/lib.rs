//! The octobot event stream: its protocol, and a client that turns it into a stream of
//! change hints.

pub mod client;
pub mod sign;

pub mod pb {
    tonic::include_proto!("octobot.events.v1");
}

/// SSH signature namespace, so a signature made here is useless to other SSH signature consumers.
pub const NAMESPACE: &str = "octobot-events";

pub const USER_HEADER: &str = "x-octobot-user";
pub const NONCE_HEADER: &str = "x-octobot-nonce-bin";
pub const SIG_HEADER: &str = "x-octobot-sig-bin";

/// What a client signs to subscribe. The authority stops a signature made for one server from
/// being relayed to another.
pub fn signed_message(authority: &str, nonce: &[u8]) -> Vec<u8> {
    let mut msg = format!("octobot-events-v1|{}|", authority).into_bytes();
    msg.extend_from_slice(nonce);
    msg
}
