use time;

fn escape_for_slack(str: &str) -> String {
    str.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
}

pub fn make_link(url: &str, text: &str) -> String {
    format!("<{}|{}>", escape_for_slack(url), escape_for_slack(text))
}

fn find_github_username(name: &str) -> Option<&str> {
    if name.len() == 0 { return None }

    for (pos, character) in name.char_indices() {
        //All characters in usernames must be alphanumeric,
        //with the exception of '-'
        if !character.is_alphanumeric() && character != '-' {
            return Some(name.split_at(pos).0)
        }
    }
    Some(name)
}

pub fn get_mentioned_usernames(body: &str) -> Vec<&str> {
    let mut mentions = Vec::new();
    for token in body.split_whitespace() {
        if token.starts_with("@") && token.len() > 1 {
            if let Some(username) = find_github_username(token.split_at(1).1) {
                mentions.push(username);
            }
        }
    }
    mentions
}

pub fn format_duration(dur: time::Duration) -> String {
    let seconds = dur.num_seconds();
    // get ms as a float
    let ms = match dur.num_microseconds() {
        Some(micro) => micro as f64 / 1000 as f64,
        None => dur.num_milliseconds() as f64,
    };
    if seconds > 0 {
        format!("{} s, {:.4} ms", seconds, (ms - (1000 * seconds) as f64))
    } else {
        format!("{:.4} ms", ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_link() {
        assert_eq!("<http://the-url|the text>",
                   make_link("http://the-url", "the text"));
    }

    #[test]
    fn test_make_link_escapes() {
        assert_eq!("<http://the-url&amp;hello=&lt;&gt;|the text &amp; &lt;&gt; stuff>",
                   make_link("http://the-url&hello=<>", "the text & <> stuff"));
    }

    #[test]
    fn test_find_github_username() {
        assert_eq!(Some("user"), find_github_username("user"));
        assert_eq!(Some("user"), find_github_username("user,"));
        assert_eq!(Some("user"), find_github_username("user,junk"));
        assert_eq!(Some("user-tanium"), find_github_username("user-tanium"));
        assert_eq!(Some("a"), find_github_username("a"));
        assert_eq!(Some("a"), find_github_username("a,"));
        assert_eq!(None, find_github_username(""));
    }

    #[test]
    fn test_mentioned_users() {
        assert_eq!(vec!["mentioned-user", "other-mentioned-user"],
                   get_mentioned_usernames("Hey @mentioned-user, let me know what @other-mentioned-user thinks"));
        assert_eq!(Vec::<&str>::new(),
                   get_mentioned_usernames("This won't count as a mention@notamention"));

    }
}
