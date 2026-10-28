use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use octobot_lib::slack::SlackRecipient;
use tokio::time::Instant;

use crate::slack::{self, SlackAttachment, SlackRequest, Threading};
use crate::util;
use crate::worker::Worker;

// Slack rejects a message with more attachments than this.
const MAX_ATTACHMENTS: usize = 100;

pub struct ReviewItem {
    review_id: u64,
    default_msg: String,
    part: Part,
}

enum Part {
    Comment(u64, SlackAttachment),
    Submitted,
    Commented,
}

impl ReviewItem {
    pub fn comment(
        review_id: u64,
        comment_id: u64,
        default_msg: &str,
        comment: SlackAttachment,
    ) -> ReviewItem {
        ReviewItem {
            review_id,
            default_msg: default_msg.to_string(),
            part: Part::Comment(comment_id, comment),
        }
    }

    pub fn submitted(review_id: u64, default_msg: &str) -> ReviewItem {
        ReviewItem {
            review_id,
            default_msg: default_msg.to_string(),
            part: Part::Submitted,
        }
    }

    pub fn commented(review_id: u64, default_msg: &str) -> ReviewItem {
        ReviewItem {
            review_id,
            default_msg: default_msg.to_string(),
            part: Part::Commented,
        }
    }
}

/// Holds review events for users who group review DMs and sends one DM per (review, user) once
/// the review has had no new events for `quiet`, or `max_wait` after its first event.
/// Held events live only in memory and are lost on restart.
pub struct ReviewGroups {
    slack: Arc<dyn Worker<SlackRequest>>,
    quiet: Duration,
    max_wait: Duration,
    pending: Mutex<HashMap<(u64, String), Pending>>,
}

struct Pending {
    recipient: SlackRecipient,
    threading: Threading,
    default_msg: String,
    header: Option<(String, Vec<SlackAttachment>)>,
    comments: Vec<(u64, SlackAttachment)>,
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

    /// Must be called from within a tokio runtime.
    /// A submitted item's `msg` and `attachments` become the head of the grouped DM.
    /// A commented item's `attachments` do too, under its `default_msg`.
    /// Comments are listed in comment id order, not arrival order.
    pub fn add(
        self: &Arc<Self>,
        item: &ReviewItem,
        recipient: SlackRecipient,
        threading: Threading,
        msg: &str,
        attachments: &[SlackAttachment],
    ) {
        let key = (item.review_id, recipient.id.clone());
        let (generation, delay) = {
            let mut pending = self.pending.lock().unwrap();
            let entry = pending.entry(key.clone()).or_insert_with(|| Pending {
                recipient,
                threading,
                default_msg: item.default_msg.clone(),
                header: None,
                comments: vec![],
                started: Instant::now(),
                generation: 0,
            });

            match &item.part {
                Part::Comment(id, comment) => entry.comments.push((*id, comment.clone())),
                Part::Submitted => entry.header = Some((msg.to_string(), attachments.to_vec())),
                Part::Commented => {
                    entry.header = Some((item.default_msg.clone(), attachments.to_vec()))
                }
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
        let mut comments = entry.comments;
        comments.sort_by_key(|(id, _)| *id);
        let listed = if attachments.len() + comments.len() <= MAX_ATTACHMENTS {
            comments.len()
        } else {
            MAX_ATTACHMENTS.saturating_sub(attachments.len() + 1)
        };
        let hidden = comments.len() - listed;
        let more = comments.get(listed).map(|(_, first_hidden)| {
            let text = format!("...and {} more comments", hidden);
            match &first_hidden.title_link {
                Some(url) => util::make_link(url, &text),
                None => text,
            }
        });
        attachments.extend(
            comments
                .into_iter()
                .take(listed)
                .map(|(_, comment)| comment),
        );
        if let Some(more) = more {
            attachments.push(SlackAttachment::new(&more));
        }

        self.slack.send(slack::req(
            entry.recipient,
            &msg,
            &attachments,
            entry.threading,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::SlackAttachmentBuilder;

    struct Capture {
        reqs: Mutex<Vec<SlackRequest>>,
    }

    impl Worker<SlackRequest> for Capture {
        fn send(&self, req: SlackRequest) {
            self.reqs.lock().unwrap().push(req);
        }
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
        ReviewItem::comment(1, 0, "carol reviewed PR", SlackAttachment::new(text))
    }

    fn user() -> SlackRecipient {
        SlackRecipient::new("U1", "alice")
    }

    #[tokio::test(start_paused = true)]
    async fn test_groups_comments_and_review_into_one_message() {
        let (capture, groups) = new_test(50, 1000);
        let state = SlackAttachment::new("Review: Changes Requested");

        groups.add(
            &comment("one"),
            user(),
            Threading::Flat,
            "Comment on PR",
            &[],
        );
        groups.add(
            &comment("two"),
            user(),
            Threading::Flat,
            "Comment on PR",
            &[],
        );
        groups.add(
            &ReviewItem::submitted(1, "carol reviewed PR"),
            user(),
            Threading::Flat,
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
                Threading::Flat,
            )],
            *capture.reqs.lock().unwrap()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_comments_only_use_default_message_and_thread() {
        let (capture, groups) = new_test(20, 1000);

        groups.add(
            &comment("one"),
            user(),
            Threading::Root("org/repo/1".into()),
            "Comment on PR",
            &[],
        );

        wait_ms(80).await;
        assert_eq!(
            vec![slack::req(
                user(),
                "carol reviewed PR",
                &[SlackAttachment::new("one")],
                Threading::Root("org/repo/1".into()),
            )],
            *capture.reqs.lock().unwrap()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_separate_reviews_and_users_are_separate_messages() {
        let (capture, groups) = new_test(20, 1000);
        let other = ReviewItem::comment(2, 0, "dave reviewed PR", SlackAttachment::new("other"));

        groups.add(&comment("one"), user(), Threading::Flat, "", &[]);
        groups.add(
            &comment("one"),
            SlackRecipient::new("U2", "bob"),
            Threading::Flat,
            "",
            &[],
        );
        groups.add(&other, user(), Threading::Flat, "", &[]);

        wait_ms(80).await;
        assert_eq!(3, capture.reqs.lock().unwrap().len());
    }

    #[tokio::test(start_paused = true)]
    async fn test_comments_sorted_by_id() {
        let (capture, groups) = new_test(20, 1000);

        for (id, text) in [(889, "third"), (882, "first"), (886, "second")] {
            let item = ReviewItem::comment(1, id, "", SlackAttachment::new(text));
            groups.add(&item, user(), Threading::Flat, "", &[]);
        }

        wait_ms(80).await;
        let reqs = capture.reqs.lock().unwrap();
        let texts: Vec<&str> = reqs[0]
            .attachments
            .iter()
            .map(|a| a.text.as_str())
            .collect();
        assert_eq!(vec!["first", "second", "third"], texts);
    }

    #[tokio::test(start_paused = true)]
    async fn test_caps_listed_comments() {
        let (capture, groups) = new_test(20, 1000);

        for i in 0..101 {
            let attachment = SlackAttachmentBuilder::new(&i.to_string())
                .title_link(format!("http://comment/{}", i))
                .build();
            let item = ReviewItem::comment(1, i, "", attachment);
            groups.add(&item, user(), Threading::Flat, "", &[]);
        }

        wait_ms(80).await;
        let reqs = capture.reqs.lock().unwrap();
        assert_eq!(1, reqs.len());
        assert_eq!(MAX_ATTACHMENTS, reqs[0].attachments.len());
        assert_eq!(
            "<http://comment/99|...and 2 more comments>",
            reqs[0].attachments[MAX_ATTACHMENTS - 1].text
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_commented_review_uses_default_message() {
        let (capture, groups) = new_test(20, 1000);
        let body = SlackAttachment::new("looks close");

        groups.add(&comment("one"), user(), Threading::Flat, "", &[]);
        groups.add(
            &ReviewItem::commented(1, "carol reviewed PR"),
            user(),
            Threading::Flat,
            "Comment on PR",
            std::slice::from_ref(&body),
        );

        wait_ms(80).await;
        assert_eq!(
            vec![slack::req(
                user(),
                "carol reviewed PR",
                &[body, SlackAttachment::new("one")],
                Threading::Flat,
            )],
            *capture.reqs.lock().unwrap()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_max_wait_sends_without_waiting_for_quiet() {
        let (capture, groups) = new_test(60, 100);

        for _ in 0..4 {
            groups.add(&comment("tick"), user(), Threading::Flat, "", &[]);
            wait_ms(40).await;
        }

        let reqs = capture.reqs.lock().unwrap();
        assert_eq!(1, reqs.len());
        assert_eq!(3, reqs[0].attachments.len());
    }
}
