use std::collections::BTreeSet;
use std::sync::Arc;

use octobot_events::Kind;
use tokio::sync::broadcast;

/// A hint that something on GitHub changed. Subscribers refetch it themselves.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub repo: String,
    pub kind: Kind,
    pub action: String,
    /// The pull request or issue, if there is one
    pub number: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub struct Event {
    /// Lowercase GitHub logins
    pub recipients: BTreeSet<String>,
    pub change: Change,
}

impl Event {
    pub fn is_for(&self, login: &str) -> bool {
        self.recipients.contains(&login.to_lowercase())
    }
}

/// Fans changes out to live subscribers. Nothing is kept for subscribers that connect later.
pub struct EventBus {
    sender: broadcast::Sender<Arc<Event>>,
}

impl EventBus {
    /// A receiver that falls `lag_capacity` events behind sees `RecvError::Lagged`.
    pub fn new(lag_capacity: usize) -> EventBus {
        let (sender, _) = broadcast::channel(lag_capacity);
        EventBus { sender }
    }

    pub fn publish(&self, recipients: impl IntoIterator<Item = String>, change: Change) {
        let recipients: BTreeSet<String> = recipients
            .into_iter()
            .map(|login| login.to_lowercase())
            .collect();
        if recipients.is_empty() {
            return;
        }
        // An error only means nobody is subscribed.
        let _ = self.sender.send(Arc::new(Event { recipients, change }));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(number: u64) -> Change {
        Change {
            repo: "some-org/some-repo".into(),
            kind: Kind::PullRequest,
            action: "closed".into(),
            number: Some(number),
        }
    }

    #[test]
    fn test_recipients_match_case_insensitively() {
        let bus = EventBus::new(10);
        let mut receiver = bus.subscribe();
        bus.publish(vec!["Joe".to_string()], change(1));

        let event = receiver.try_recv().unwrap();
        assert!(event.is_for("joe"));
        assert!(event.is_for("JOE"));
        assert!(!event.is_for("bob"));
    }

    #[test]
    fn test_no_recipients_publishes_nothing() {
        let bus = EventBus::new(10);
        let mut receiver = bus.subscribe();
        bus.publish(Vec::new(), change(1));

        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_subscriber_sees_only_later_events() {
        let bus = EventBus::new(10);
        bus.publish(vec!["joe".to_string()], change(1));
        let mut receiver = bus.subscribe();
        bus.publish(vec!["joe".to_string()], change(2));

        assert_eq!(change(2), receiver.try_recv().unwrap().change);
        assert!(receiver.try_recv().is_err());
    }
}
