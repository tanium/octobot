use crate::slack::Slack;
use octobot_lib::config::Config;
use octobot_lib::errors::*;

pub async fn migrate_slack_id(config: &Config, slack: &Slack) {
    match do_migrate_slack_id(config, slack).await {
        Ok(count) => {
            if count > 0 {
                log::info!("Migrated {} slack users", count);
            }
        }
        Err(e) => {
            log::error!("Failed to migrate slack users: {}", e);
        }
    }
}

async fn do_migrate_slack_id(config: &Config, slack: &Slack) -> Result<u32> {
    let mut all_users = config
        .users()
        .get_all()?
        .into_iter()
        .filter(|u| u.slack_id.is_empty())
        .collect::<Vec<_>>();

    if all_users.is_empty() {
        return Ok(0);
    }

    let all_slack_users = slack.list_users().await?;

    let mut migrated = 0;
    for user in &mut all_users {
        for slack in &all_slack_users {
            if user.slack_name == slack.name {
                user.slack_id = slack.id.clone();
                user.email = slack.profile.email.clone();

                if let Err(e) = config.users_write().update(user) {
                    log::error!(
                        "Failed to migrate slack name: {} -> {}: {}",
                        slack.name,
                        slack.id,
                        e
                    );
                } else {
                    log::info!("Migrated slack name: {} -> {}", slack.name, slack.id);
                    migrated += 1;
                }
            }
        }
    }

    for user in &all_users {
        if user.slack_id.is_empty() {
            log::info!("No matched slack username: {}", user.slack_name)
        }
    }

    Ok(migrated)
}
