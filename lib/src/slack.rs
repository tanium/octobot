// A slack recipient represents either a channel or a user by name/id
#[derive(Debug, PartialEq, Clone)]
pub struct SlackRecipient {
    pub id: String,
    pub name: String,
}

impl SlackRecipient {
    pub fn new(id: &str, name: &str) -> SlackRecipient {
        SlackRecipient {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    pub fn by_name(name: &str) -> SlackRecipient {
        SlackRecipient {
            id: name.to_string(),
            name: name.to_string(),
        }
    }

    pub fn user_mention(name: &str) -> SlackRecipient {
        SlackRecipient {
            id: format!("@{}", name),
            name: name.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mention() {
        let channel = SlackRecipient::user_mention("me");
        assert_eq!("@me", channel.id);
        assert_eq!("me", channel.name);
    }
}
