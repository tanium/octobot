//! The octobot event stream: its protocol, and a client that turns it into a stream of
//! change hints.

use tonic::codegen::http::Uri;

pub mod client;
pub mod sign;

pub mod pb {
    tonic::include_proto!("octobot.events.v1");
}

/// The GitHub webhook events octobot publishes hints for
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    PullRequest,
    PullRequestReview,
    PullRequestReviewComment,
    CommitComment,
    IssueComment,
    Push,
    CheckRun,
    CheckSuite,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::PullRequest,
        Kind::PullRequestReview,
        Kind::PullRequestReviewComment,
        Kind::CommitComment,
        Kind::IssueComment,
        Kind::Push,
        Kind::CheckRun,
        Kind::CheckSuite,
    ];

    /// From an X-GitHub-Event value; None for events octobot publishes no hints for.
    pub fn from_event(event: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.as_event() == event)
    }

    /// The X-GitHub-Event value
    pub fn as_event(self) -> &'static str {
        match self {
            Kind::PullRequest => "pull_request",
            Kind::PullRequestReview => "pull_request_review",
            Kind::PullRequestReviewComment => "pull_request_review_comment",
            Kind::CommitComment => "commit_comment",
            Kind::IssueComment => "issue_comment",
            Kind::Push => "push",
            Kind::CheckRun => "check_run",
            Kind::CheckSuite => "check_suite",
        }
    }

    /// None for `KIND_UNSPECIFIED`, or a kind from a newer server.
    pub fn from_pb(kind: i32) -> Option<Kind> {
        Some(match pb::Kind::try_from(kind).ok()? {
            pb::Kind::Unspecified => return None,
            pb::Kind::PullRequest => Kind::PullRequest,
            pb::Kind::PullRequestReview => Kind::PullRequestReview,
            pb::Kind::PullRequestReviewComment => Kind::PullRequestReviewComment,
            pb::Kind::CommitComment => Kind::CommitComment,
            pb::Kind::IssueComment => Kind::IssueComment,
            pb::Kind::Push => Kind::Push,
            pb::Kind::CheckRun => Kind::CheckRun,
            pb::Kind::CheckSuite => Kind::CheckSuite,
        })
    }
}

impl From<Kind> for pb::Kind {
    fn from(kind: Kind) -> pb::Kind {
        match kind {
            Kind::PullRequest => pb::Kind::PullRequest,
            Kind::PullRequestReview => pb::Kind::PullRequestReview,
            Kind::PullRequestReviewComment => pb::Kind::PullRequestReviewComment,
            Kind::CommitComment => pb::Kind::CommitComment,
            Kind::IssueComment => pb::Kind::IssueComment,
            Kind::Push => pb::Kind::Push,
            Kind::CheckRun => pb::Kind::CheckRun,
            Kind::CheckSuite => pb::Kind::CheckSuite,
        }
    }
}

/// SSH signature namespace, so a signature made here is useless to other SSH signature consumers.
pub const NAMESPACE: &str = "octobot-events";

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

    #[test]
    fn test_kind_from_event() {
        assert_eq!(Some(Kind::PullRequest), Kind::from_event("pull_request"));
        assert_eq!(Some(Kind::Push), Kind::from_event("push"));
        assert_eq!(Some(Kind::CheckSuite), Kind::from_event("check_suite"));
        assert_eq!(None, Kind::from_event("ping"));
        for kind in Kind::ALL {
            assert_eq!(Some(kind), Kind::from_event(kind.as_event()));
        }
    }

    #[test]
    fn test_kind_round_trips_through_the_proto() {
        for kind in Kind::ALL {
            assert_eq!(Some(kind), Kind::from_pb(pb::Kind::from(kind) as i32));
        }
        assert_eq!(None, Kind::from_pb(pb::Kind::Unspecified as i32));
        assert_eq!(None, Kind::from_pb(99));
    }

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
