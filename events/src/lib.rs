//! The octobot event stream: its protocol, and a client that turns it into a stream of
//! change hints.

use tonic::codegen::http::Uri;

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

/// The authority as signed: the host lowercased, with its port only when it is not the
/// scheme's default. The client applies it to its endpoint and the server to its configured
/// authority, so the two compare equal as strings.
pub fn signed_authority(uri: &Uri) -> anyhow::Result<String> {
    let host = uri
        .host()
        .ok_or_else(|| anyhow::anyhow!("no host in {}", uri))?
        .to_lowercase();
    let default_port = match uri.scheme_str() {
        Some("https") => Some(443),
        Some("http") => Some(80),
        _ => None,
    };
    Ok(match uri.port_u16() {
        Some(port) if Some(port) != default_port => format!("{}:{}", host, port),
        _ => host,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority(endpoint: &str) -> String {
        signed_authority(&endpoint.parse().unwrap()).unwrap()
    }

    #[test]
    fn test_signed_authority_drops_default_port() {
        assert_eq!(
            "octobot.example.com",
            authority("https://octobot.example.com")
        );
        assert_eq!(
            "octobot.example.com",
            authority("https://octobot.example.com:443")
        );
        assert_eq!(
            "octobot.example.com",
            authority("http://octobot.example.com:80")
        );
    }

    #[test]
    fn test_signed_authority_keeps_other_ports() {
        assert_eq!(
            "octobot.example.com:3001",
            authority("https://octobot.example.com:3001")
        );
        assert_eq!(
            "octobot.example.com:443",
            authority("http://octobot.example.com:443")
        );
    }

    #[test]
    fn test_signed_authority_lowercases_host() {
        assert_eq!(
            "octobot.example.com",
            authority("https://Octobot.Example.com")
        );
    }
}
