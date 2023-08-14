use std::collections::BTreeMap;
use std::sync::Arc;

use crate::slack::{self, SlackAttachment, SlackRequest};
use crate::util;
use crate::worker::Worker;
use octobot_lib::config::Config;
use octobot_lib::github;
use octobot_lib::slack::SlackRecipient;

pub struct Messenger {
    config: Arc<Config>,
    slack: Arc<dyn Worker<SlackRequest>>,
}

#[derive(PartialEq, Clone, Debug)]
pub enum ParticipantType {
    User,
    TeamMember,
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

    pub fn add_team_member(&mut self, user: github::User) {
        self.add(Participant {
            user,
            participant_type: ParticipantType::TeamMember,
        });
    }

    fn add(&mut self, p: Participant) {
        let new_type = p.participant_type.clone();

        let entry = self.users.entry(p.user.login().to_string()).or_insert(p);

        if entry.participant_type != new_type {
            match new_type {
                ParticipantType::User => {
                    // override.
                    entry.participant_type = new_type
                }
                ParticipantType::TeamMember => {
                    // ignore. team members do not override
                }
            }
        }
    }
}

pub fn new(config: Arc<Config>, slack: Arc<dyn Worker<SlackRequest>>) -> Messenger {
    Messenger {
        slack: slack.clone(),
        config,
    }
}

impl Messenger {
    // TODO
    #[allow(clippy::too_many_arguments)]
    pub fn send_to_all<T: github::CommitLike>(
        &self,
        msg: &str,
        attachments: &[SlackAttachment],
        item_owner: &github::User,
        sender: &github::User,
        repo: &github::Repo,
        mut participants: Participants,
        branch: &str,
        commits: &[T],
        thread_guids: Vec<String>,
    ) {
        self.send_to_channel(msg, attachments, repo, branch, commits, thread_guids, false);

        participants.add_user(item_owner.clone());

        // make sure we do not send private message to author of that message
        participants.remove(sender.login());
        participants.remove("octobot");

        self.send_to_slackbots(participants, repo, msg, attachments);
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
        self.send_to_channel(
            msg,
            attachments,
            repo,
            branch,
            commits,
            Vec::<String>::new(),
            false,
        );
        self.send_to_slackbots(
            Participants::single(item_owner.clone()),
            repo,
            msg,
            attachments,
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
        thread_guids: Vec<String>,
        initial_thread: bool,
    ) {
        // We can use the thread_guid to see if threads have been used in the past within the
        //  slack db, but that wouldn't respect the users' choice if they change the setting later.
        let use_threads = self.config.repos().notify_use_threads(repo) && !thread_guids.is_empty();

        for channel in self.config.repos().lookup_channels(repo, branch, commits) {
            let channel_msg = format!(
                "{} ({})",
                msg,
                util::make_link(&repo.html_url, &repo.full_name)
            );
            if !use_threads {
                self.slack.send(slack::req(
                    SlackRecipient::new(&channel, &channel),
                    &channel_msg,
                    attachments,
                    None,
                    initial_thread,
                ));
            } else {
                for thread_guid in &thread_guids {
                    self.slack.send(slack::req(
                        SlackRecipient::new(&channel, &channel),
                        &channel_msg,
                        attachments,
                        Some(thread_guid.to_owned()),
                        initial_thread,
                    ));
                }
            }
        }
    }

    fn send_to_slackbots(
        &self,
        users: Participants,
        repo: &github::Repo,
        msg: &str,
        attachments: &[SlackAttachment],
    ) {
        for (_, user) in users.users.iter() {
            let is_team_member = match user.participant_type {
                ParticipantType::User => false,
                ParticipantType::TeamMember => true,
            };

            let user_dm = self.config.users().slack_direct_message(
                user.login(),
                is_team_member,
                &repo.full_name,
            );

            if let Some(user_dm) = user_dm {
                self.slack
                    .send(slack::req(user_dm, msg, attachments, None, false));
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
        p.add_team_member(github::User::new("a"));

        let entry = p.users.get("a").unwrap();
        assert_eq!(entry.participant_type, ParticipantType::User);
    }

    #[test]
    fn test_participation_type_priority2() {
        let mut p = Participants::new();

        p.add_team_member(github::User::new("a"));
        p.add_user(github::User::new("a"));

        let entry = p.users.get("a").unwrap();
        assert_eq!(entry.participant_type, ParticipantType::User);
    }
}
