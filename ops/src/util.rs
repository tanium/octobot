use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::anyhow;

use octobot_lib::errors::*;

fn escape_for_slack(str: &str) -> String {
    str.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn make_link(url: &str, text: &str) -> String {
    format!("<{}|{}>", escape_for_slack(url), escape_for_slack(text))
}

fn find_github_username(name: &str) -> Option<&str> {
    if name.is_empty() {
        return None;
    }

    for (pos, character) in name.char_indices() {
        //All characters in usernames must be alphanumeric,
        //with the exception of '-'
        if !character.is_alphanumeric() && character != '-' {
            return Some(name.split_at(pos).0);
        }
    }
    Some(name)
}

pub fn get_mentioned_usernames(body: &str) -> Vec<&str> {
    let mut mentions = Vec::new();
    for token in body.split_whitespace() {
        if token.starts_with('@') && token.len() > 1 {
            if let Some(username) = find_github_username(token.split_at(1).1) {
                mentions.push(username);
            }
        }
    }
    mentions
}

pub fn format_duration(dur: std::time::Duration) -> String {
    let seconds = dur.as_secs();
    let ms = (dur.subsec_micros() as f64) / 1000_f64;
    if seconds > 0 {
        format!("{} s, {:.4} ms", seconds, ms)
    } else {
        format!("{:.4} ms", ms)
    }
}

pub fn check_unique_event<T>(event: T, events: &mut Vec<T>, trim_at: usize, trim_to: usize) -> bool
where
    T: PartialEq,
{
    let unique = !events.contains(&event);

    if unique {
        events.push(event);
        trim_unique_events(events, trim_at, trim_to);
    }

    unique
}

pub fn trim_unique_events<T>(events: &mut Vec<T>, trim_at: usize, trim_to: usize) {
    if events.len() > trim_at {
        // reverse so that that we keep recent events
        events.reverse();
        events.truncate(trim_to);
        events.reverse();
    }
}

pub fn parse_query(query_params: Option<&str>) -> HashMap<String, String> {
    if query_params.is_none() {
        return HashMap::new();
    }
    query_params
        .unwrap()
        .split('&')
        .filter_map(|v| {
            let parts = v.splitn(2, '=').collect::<Vec<_>>();
            if parts.len() != 2 {
                None
            } else {
                Some((parts[0].to_string(), parts[1].to_string()))
            }
        })
        .collect::<HashMap<_, _>>()
}

// cf. https://github.com/rust-lang/rust/issues/39364
pub fn recv_timeout<T>(rx: &Receiver<T>, timeout: std::time::Duration) -> Result<T> {
    let sleep_time = std::time::Duration::from_millis(50);
    let mut time_left = timeout;
    loop {
        match rx.try_recv() {
            Ok(r) => {
                return Ok(r);
            }
            Err(mpsc::TryRecvError::Empty) => match time_left.checked_sub(sleep_time) {
                Some(sub) => {
                    time_left = sub;
                    thread::sleep(sleep_time);
                }
                None => {
                    return Err(anyhow!("Timed out waiting"));
                }
            },
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(anyhow!("Channel disconnected!"));
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maplit::hashmap;

    #[test]
    fn test_make_link() {
        assert_eq!(
            "<http://the-url|the text>",
            make_link("http://the-url", "the text")
        );
    }

    #[test]
    fn test_make_link_escapes() {
        assert_eq!(
            "<http://the-url&amp;hello=&lt;&gt;|the text &amp; &lt;&gt; stuff>",
            make_link("http://the-url&hello=<>", "the text & <> stuff")
        );
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
        assert_eq!(
            vec!["mentioned-user", "other-mentioned-user"],
            get_mentioned_usernames(
                "Hey @mentioned-user, let me know what @other-mentioned-user thinks"
            )
        );
        assert_eq!(
            Vec::<&str>::new(),
            get_mentioned_usernames("This won't count as a mention@notamention")
        );
    }

    #[test]
    fn test_check_unique_event() {
        let trim_at = 5;
        let trim_to = 2;
        let mut events: Vec<String> = vec![];

        assert!(check_unique_event(
            "A".into(),
            &mut events,
            trim_at,
            trim_to
        ));
        assert_eq!(vec!["A"], events);

        assert!(check_unique_event(
            "B".into(),
            &mut events,
            trim_at,
            trim_to
        ));
        assert_eq!(vec!["A", "B"], events);
        assert!(!check_unique_event(
            "B".into(),
            &mut events,
            trim_at,
            trim_to
        ));
        assert_eq!(vec!["A", "B"], events);

        assert!(check_unique_event(
            "C".into(),
            &mut events,
            trim_at,
            trim_to
        ));
        assert!(check_unique_event(
            "D".into(),
            &mut events,
            trim_at,
            trim_to
        ));
        assert!(check_unique_event(
            "E".into(),
            &mut events,
            trim_at,
            trim_to
        ));
        assert_eq!(vec!["A", "B", "C", "D", "E"], events);

        // next one should trigger a trim!
        assert!(check_unique_event(
            "F".into(),
            &mut events,
            trim_at,
            trim_to
        ));
        assert_eq!(vec!["E", "F"], events);
    }

    #[test]
    fn test_parse_query() {
        let map = hashmap! {
            "A".to_string() => "1".to_string(),
            "B".to_string() => "Hello%20There".to_string(),
        };

        assert_eq!(map, parse_query(Some("A=1&B=Hello%20There")));
    }

    #[test]
    fn test_parse_query_none() {
        assert_eq!(HashMap::new(), parse_query(None));
    }
}
