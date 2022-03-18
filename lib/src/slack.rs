#[derive(Debug, PartialEq, Clone)]
pub struct SlackChannel {
    pub id: String,
    pub name: String,
}

impl SlackChannel {
    pub fn new(id: &str, name: &str) -> SlackChannel {
        SlackChannel {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    pub fn by_name(name: &str) -> SlackChannel {
        SlackChannel {
            id: name.to_string(),
            name: name.to_string(),
        }
    }
}
