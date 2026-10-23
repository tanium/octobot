use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use octobot_lib::slack::SlackRecipient;

use crate::slack::{self, SlackAttachment, SlackRequest};
use crate::worker::Worker;

const MAX_COMMENTS: usize = 10;

pub struct ReviewItem {
    review_id: u64,
    default_msg: String,
    comment: Option<SlackAttachment>,
}

impl ReviewItem {
    pub fn comment(review_id: u64, default_msg: &str, comment: SlackAttachment) -> ReviewItem {
        ReviewItem {
            review_id,
            default_msg: default_msg.to_string(),
            comment: Some(comment),
        }
    }

    pub fn submitted(review_id: u64, default_msg: &str) -> ReviewItem {
        ReviewItem {
            review_id,
            default_msg: default_msg.to_string(),
            comment: None,
        }
    }
}

/**
 * Holds review events for users who group review DMs and sends one DM per (review, user) once
 * the review has had no new events for `quiet`, or `max_wait` after its first event.
 * Held events live only in memory and are lost on restart.
 */
pub struct ReviewGroups {
    slack: Arc<dyn Worker<SlackRequest>>,
    quiet: Duration,
    max_wait: Duration,
    pending: Mutex<HashMap<(u64, String), Pending>>,
}

struct Pending {
    recipient: SlackRecipient,
    thread_guid: Option<String>,
    default_msg: String,
    header: Option<(String, Vec<SlackAttachment>)>,
    comments: Vec<SlackAttachment>,
    started: Instant,
    generation: u64,
}

impl ReviewGroups {
    pub fn new(
        slack: Arc<dyn Worker<SlackRequest>>,
        quiet: Duration,
        max_wait: Duration,
    ) -> Arc<ReviewGroups> {
        Arc::new(ReviewGroups {
            slack,
            quiet,
            max_wait,
            pending: Mutex::new(HashMap::new()),
        })
    }

    /**
     * Must be called from within a tokio runtime.
     * A submitted item's `msg` and `attachments` become the head of the grouped DM.
     */
    pub fn add(
        self: &Arc<Self>,
        item: &ReviewItem,
        recipient: SlackRecipient,
        thread_guid: Option<String>,
        msg: &str,
        attachments: &[SlackAttachment],
    ) {
        let key = (item.review_id, recipient.id.clone());
        let (generation, delay) = {
            let mut pending = self.pending.lock().unwrap();
            let entry = pending.entry(key.clone()).or_insert_with(|| Pending {
                recipient,
                thread_guid,
                default_msg: item.default_msg.clone(),
                header: None,
                comments: vec![],
                started: Instant::now(),
                generation: 0,
            });

            match &item.comment {
                Some(comment) => entry.comments.push(comment.clone()),
                None => entry.header = Some((msg.to_string(), attachments.to_vec())),
            }
            entry.generation += 1;

            let remaining = self.max_wait.saturating_sub(entry.started.elapsed());
            (entry.generation, self.quiet.min(remaining))
        };

        let groups = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            groups.flush(&key, generation);
        });
    }

    fn flush(&self, key: &(u64, String), generation: u64) {
        let entry = {
            let mut pending = self.pending.lock().unwrap();
            match pending.get(key) {
                Some(p) if p.generation == generation => pending.remove(key).unwrap(),
                _ => return,
            }
        };

        let (msg, mut attachments) = entry.header.unwrap_or((entry.default_msg, vec![]));
        let hidden = entry.comments.len().saturating_sub(MAX_COMMENTS);
        attachments.extend(entry.comments.into_iter().take(MAX_COMMENTS));
        if hidden > 0 {
            attachments.push(SlackAttachment::new(&format!(
                "...and {} more comments",
                hidden
            )));
        }

        let initial_thread = entry.thread_guid.is_some();
        self.slack.send(slack::req(
            entry.recipient,
            &msg,
            &attachments,
            entry.thread_guid,
            initial_thread,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Capture {
        reqs: Mutex<Vec<SlackRequest>>,
    }

    impl Worker<SlackRequest> for Capture {
        fn send(&self, req: SlackRequest) {
            self.reqs.lock().unwrap().push(req);
        }
    }

    fn run<F: std::future::Future<Output = ()>>(f: F) {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(f);
    }

    fn new_test(quiet_ms: u64, max_wait_ms: u64) -> (Arc<Capture>, Arc<ReviewGroups>) {
        let capture = Arc::new(Capture {
            reqs: Mutex::new(vec![]),
        });
        let groups = ReviewGroups::new(
            capture.clone(),
            Duration::from_millis(quiet_ms),
            Duration::from_millis(max_wait_ms),
        );
        (capture, groups)
    }

    async fn wait_ms(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    fn comment(text: &str) -> ReviewItem {
        ReviewItem::comment(1, "carol reviewed PR", SlackAttachment::new(text))
    }

    fn user() -> SlackRecipient {
        SlackRecipient::new("U1", "alice")
    }

    #[test]
    fn test_groups_comments_and_review_into_one_message() {
        run(async {
            let (capture, groups) = new_test(50, 1000);
            let state = SlackAttachment::new("Review: Changes Requested");

            groups.add(&comment("one"), user(), None, "Comment on PR", &[]);
            groups.add(&comment("two"), user(), None, "Comment on PR", &[]);
            groups.add(
                &ReviewItem::submitted(1, "carol reviewed PR"),
                user(),
                None,
                "carol requested changes to PR",
                std::slice::from_ref(&state),
            );

            wait_ms(20).await;
            assert!(capture.reqs.lock().unwrap().is_empty());

            wait_ms(100).await;
            assert_eq!(
                vec![slack::req(
                    user(),
                    "carol requested changes to PR",
                    &[
                        state,
                        SlackAttachment::new("one"),
                        SlackAttachment::new("two")
                    ],
                    None,
                    false,
                )],
                *capture.reqs.lock().unwrap()
            );
        });
    }

    #[test]
    fn test_comments_only_use_default_message_and_thread() {
        run(async {
            let (capture, groups) = new_test(20, 1000);

            groups.add(
                &comment("one"),
                user(),
                Some("org/repo/1".into()),
                "Comment on PR",
                &[],
            );

            wait_ms(80).await;
            assert_eq!(
                vec![slack::req(
                    user(),
                    "carol reviewed PR",
                    &[SlackAttachment::new("one")],
                    Some("org/repo/1".into()),
                    true,
                )],
                *capture.reqs.lock().unwrap()
            );
        });
    }

    #[test]
    fn test_separate_reviews_and_users_are_separate_messages() {
        run(async {
            let (capture, groups) = new_test(20, 1000);
            let other = ReviewItem::comment(2, "dave reviewed PR", SlackAttachment::new("other"));

            groups.add(&comment("one"), user(), None, "", &[]);
            groups.add(
                &comment("one"),
                SlackRecipient::new("U2", "bob"),
                None,
                "",
                &[],
            );
            groups.add(&other, user(), None, "", &[]);

            wait_ms(80).await;
            assert_eq!(3, capture.reqs.lock().unwrap().len());
        });
    }

    #[test]
    fn test_caps_listed_comments() {
        run(async {
            let (capture, groups) = new_test(20, 1000);

            for i in 0..12 {
                groups.add(&comment(&i.to_string()), user(), None, "", &[]);
            }

            wait_ms(80).await;
            let reqs = capture.reqs.lock().unwrap();
            assert_eq!(1, reqs.len());
            assert_eq!(MAX_COMMENTS + 1, reqs[0].attachments.len());
            assert_eq!(
                "...and 2 more comments",
                reqs[0].attachments[MAX_COMMENTS].text
            );
        });
    }

    #[test]
    fn test_max_wait_sends_without_waiting_for_quiet() {
        run(async {
            let (capture, groups) = new_test(60, 100);

            for _ in 0..4 {
                groups.add(&comment("tick"), user(), None, "", &[]);
                wait_ms(40).await;
            }

            let reqs = capture.reqs.lock().unwrap();
            assert_eq!(1, reqs.len());
            assert_eq!(3, reqs[0].attachments.len());
        });
    }
}
