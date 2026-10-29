use log::info;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::review_group::{ReviewGroups, ReviewItem};
use crate::slack::{self, SlackAttachment, SlackRequest, ThreadRoot, Threading};
use crate::util;
use crate::worker::Worker;
use octobot_lib::config::Config;
use octobot_lib::github;
use octobot_lib::slack::SlackRecipient;

pub struct Messenger {
    config: Arc<Config>,
    slack: Arc<dyn Worker<SlackRequest>>,
    review_groups: Option<Arc<ReviewGroups>>,
}

#[derive(PartialEq, Clone, Debug)]
pub enum ParticipantType {
    User,
    TeamMember(BTreeSet<String>),
}

#[derive(PartialEq, Clone)]
pub struct Participant {
    user: github::User,
    participant_type: ParticipantType,
}

impl Participant {
    pub fn login(&self) -> &str {
        self.user.login()
    }
}

#[derive(Clone)]
pub struct Participants {
    users: BTreeMap<String, Participant>,
}

impl Participants {
    pub fn new() -> Self {
        Participants {
            users: BTreeMap::new(),
        }
    }

    pub fn single(user: github::User) -> Self {
        let mut p = Self::new();
        p.add_user(user);
        p
    }

    pub fn remove(&mut self, login: &str) {
        self.users.remove(login);
    }

    pub fn add_user(&mut self, user: github::User) {
        self.add(Participant {
            user,
            participant_type: ParticipantType::User,
        });
    }

    pub fn add_team_member(&mut self, user: github::User, team: &str) {
        self.add(Participant {
            user,
            participant_type: ParticipantType::TeamMember(BTreeSet::from([team.to_string()])),
        });
    }

    fn add(&mut self, p: Participant) {
        let Some(entry) = self.users.get_mut(p.user.login()) else {
            self.users.insert(p.user.login().to_string(), p);
            return;
        };

        match p.participant_type {
            ParticipantType::User => entry.participant_type = ParticipantType::User,
            ParticipantType::TeamMember(teams) => {
                if let ParticipantType::TeamMember(existing) = &mut entry.participant_type {
                    existing.extend(teams);
                }
            }
        }
    }
}

/// A slack thread key. When `pull_request` is set and a channel has no thread for the key yet,
/// a "New thread for Pull Request" message is posted first to start one.
#[derive(Clone)]
pub struct Thread {
    guid: String,
    pull_request: Option<SlackAttachment>,
}

impl Thread {
    pub fn new(guid: &str) -> Thread {
        Thread {
            guid: guid.to_string(),
            pull_request: None,
        }
    }

    pub fn for_pull_request(guid: &str, pull_request: SlackAttachment) -> Thread {
        Thread {
            guid: guid.to_string(),
            pull_request: Some(pull_request),
        }
    }
}

pub fn new(config: Arc<Config>, slack: Arc<dyn Worker<SlackRequest>>) -> Messenger {
    Messenger {
        slack: slack.clone(),
        config,
        review_groups: None,
    }
}

impl Messenger {
    pub fn with_review_groups(self, review_groups: Arc<ReviewGroups>) -> Messenger {
        Messenger {
            review_groups: Some(review_groups),
            ..self
        }
    }

    // TODO
    #[allow(clippy::too_many_arguments)]
    pub fn send_to_all<T: github::CommitLike>(
        &self,
        msg: &str,
        attachments: &[SlackAttachment],
        item_owner: &github::User,
        sender: &github::User,
        repo: &github::Repo,
        participants: Participants,
        branch: &str,
        commits: &[T],
        threads: Vec<Thread>,
    ) {
        self.send_review_to_all(
            None,
            msg,
            attachments,
            item_owner,
            sender,
            repo,
            participants,
            branch,
            commits,
            threads,
        );
    }

    /// Like `send_to_all`. With a `review`, DMs to users who group review messages are held and
    /// sent as one message per review.
    #[allow(clippy::too_many_arguments)]
    pub fn send_review_to_all<T: github::CommitLike>(
        &self,
        review: Option<&ReviewItem>,
        msg: &str,
        attachments: &[SlackAttachment],
        item_owner: &github::User,
        sender: &github::User,
        repo: &github::Repo,
        mut participants: Participants,
        branch: &str,
        commits: &[T],
        threads: Vec<Thread>,
    ) {
        if self.is_ignored_user(sender) {
            info!("Ignoring message from ignored user: {}", sender.login(),);
            return;
        }

        // A commit can belong to several PRs; one DM thread is enough.
        let dm_thread_guid = threads
            .iter()
            .find(|t| t.pull_request.is_some())
            .map(|t| t.guid.clone());

        self.send_to_channel(msg, attachments, repo, branch, commits, threads, false);

        participants.add_user(item_owner.clone());

        // make sure we do not send private message to author of that message
        participants.remove(sender.login());
        participants.remove("octobot");

        self.send_to_slackbots(participants, repo, msg, attachments, dm_thread_guid, review);
    }

    fn is_ignored_user(&self, user: &github::User) -> bool {
        self.config
            .slack
            .ignored_users
            .contains(&user.login().to_string())
    }

    pub fn send_to_owner<T: github::CommitLike>(
        &self,
        msg: &str,
        attachments: &[SlackAttachment],
        item_owner: &github::User,
        repo: &github::Repo,
        branch: &str,
        commits: &[T],
    ) {
        self.send_to_channel(msg, attachments, repo, branch, commits, Vec::new(), false);
        self.send_to_slackbots(
            Participants::single(item_owner.clone()),
            repo,
            msg,
            attachments,
            None,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_to_channel<T: github::CommitLike>(
        &self,
        msg: &str,
        attachments: &[SlackAttachment],
        repo: &github::Repo,
        branch: &str,
        commits: &[T],
        threads: Vec<Thread>,
        initial_thread: bool,
    ) {
        // We can use the thread_guid to see if threads have been used in the past within the
        //  slack db, but that wouldn't respect the users' choice if they change the setting later.
        let use_threads = self.config.repos().notify_use_threads(repo) && !threads.is_empty();
        let repo_link = util::make_link(&repo.html_url, &repo.full_name);

        for channel in self.config.repos().lookup_channels(repo, branch, commits) {
            let recipient = SlackRecipient::new(&channel, &channel);
            let channel_msg = format!("{} ({})", msg, repo_link);
            if !use_threads {
                self.slack.send(slack::req(
                    recipient,
                    &channel_msg,
                    attachments,
                    Threading::Flat,
                ));
                continue;
            }

            for thread in &threads {
                let guid = thread.guid.clone();
                let threading = match &thread.pull_request {
                    _ if initial_thread => Threading::Root(guid),
                    Some(pr) => Threading::ReplyOrStart(
                        guid,
                        ThreadRoot {
                            msg: format!("New thread for Pull Request ({})", repo_link),
                            attachments: vec![pr.clone()],
                        },
                    ),
                    None => Threading::Reply(guid),
                };
                self.slack.send(slack::req(
                    recipient.clone(),
                    &channel_msg,
                    attachments,
                    threading,
                ));
            }
        }
    }

    fn send_to_slackbots(
        &self,
        users: Participants,
        repo: &github::Repo,
        msg: &str,
        attachments: &[SlackAttachment],
        thread_guid: Option<String>,
        review: Option<&ReviewItem>,
    ) {
        for (_, user) in users.users.iter() {
            let via_teams = match &user.participant_type {
                ParticipantType::User => None,
                ParticipantType::TeamMember(teams) => Some(teams),
            };

            let user_dm =
                self.config
                    .users()
                    .slack_direct_message(user.login(), via_teams, &repo.full_name);

            if let Some(user_dm) = user_dm {
                let threading = match &thread_guid {
                    Some(guid) if user_dm.use_threads => Threading::Root(guid.clone()),
                    _ => Threading::Flat,
                };
                if user_dm.group_reviews
                    && let Some(review) = review
                    && let Some(groups) = &self.review_groups
                {
                    groups.add(review, user_dm.recipient, threading, msg, attachments);
                    continue;
                }

                self.slack
                    .send(slack::req(user_dm.recipient, msg, attachments, threading));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_participation_type_priority1() {
        let mut p = Participants::new();

        p.add_user(github::User::new("a"));
        p.add_team_member(github::User::new("a"), "org/team");

        let entry = p.users.get("a").unwrap();
        assert_eq!(entry.participant_type, ParticipantType::User);
    }

    #[test]
    fn test_participation_type_priority2() {
        let mut p = Participants::new();

        p.add_team_member(github::User::new("a"), "org/team");
        p.add_user(github::User::new("a"));

        let entry = p.users.get("a").unwrap();
        assert_eq!(entry.participant_type, ParticipantType::User);
    }

    #[test]
    fn test_team_member_collects_teams() {
        let mut p = Participants::new();

        p.add_team_member(github::User::new("a"), "org/team1");
        p.add_team_member(github::User::new("a"), "org/team2");

        let entry = p.users.get("a").unwrap();
        assert_eq!(
            entry.participant_type,
            ParticipantType::TeamMember(BTreeSet::from(["org/team1".into(), "org/team2".into()]))
        );
    }
}
