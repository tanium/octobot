use std::borrow::Borrow;
use std::sync::Arc;

use anyhow::anyhow;
use conventional::{Commit, Simple as _};
use log::{error, info};
use regex::Regex;

use crate::git::Git;
use crate::git_clone_manager::GitCloneManager;
use crate::messenger;
use crate::slack::{SlackAttachmentBuilder, SlackRequest};
use crate::worker;
use octobot_lib::config::Config;
use octobot_lib::errors::*;
use octobot_lib::github;
use octobot_lib::github::api::{GithubSessionFactory, Session};
use octobot_lib::github::CommitLike;
use octobot_lib::metrics::{self, Metrics};

#[derive(Debug, Clone, PartialEq)]
enum FollowsRef {
    PullRequest(u32),
    Commit(String),
}

struct ResolvedFollow {
    commit_sha: String,
    pr_number: Option<u32>,
    orig_base_branch: Option<String>,
}

fn parse_follows_refs(
    labels: &[github::Label],
    pr_body: Option<&str>,
    commits: &[github::Commit],
    repo_full_name: &str,
) -> Vec<FollowsRef> {
    let mut refs = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let label_pr_re = Regex::new(r"^follows-pr-(\d+)$").unwrap();
    let label_commit_re = Regex::new(r"^follows-commit-([0-9a-f]{7,40})$").unwrap();

    for label in labels {
        let name = &label.name;
        if let Some(caps) = label_pr_re.captures(name) {
            let num: u32 = caps[1].parse().unwrap();
            let r = FollowsRef::PullRequest(num);
            if seen.insert(r.clone()) {
                refs.push(r);
            }
        } else if let Some(caps) = label_commit_re.captures(name) {
            let hash = caps[1].to_string();
            let r = FollowsRef::Commit(hash);
            if seen.insert(r.clone()) {
                refs.push(r);
            }
        }
    }

    let text_pr_re = Regex::new(r"(?i)follows[\s-]+pr[\s-]+(\d+)").unwrap();
    let text_commit_re = Regex::new(r"(?i)follows[\s-]+commit[\s-]+([0-9a-f]{7,40})").unwrap();

    let escaped_name = regex::escape(repo_full_name);
    let link_pr_re = Regex::new(
        &format!(r"(?i)follows\s+https://github\.com/{}/pull/(\d+)", escaped_name),
    )
    .unwrap();
    let link_commit_re = Regex::new(
        &format!(
            r"(?i)follows\s+https://github\.com/{}/commit/([0-9a-f]{{7,40}})",
            escaped_name
        ),
    )
    .unwrap();

    let mut parse_text = |text: &str| {
        for caps in text_pr_re.captures_iter(text) {
            let num: u32 = caps[1].parse().unwrap();
            let r = FollowsRef::PullRequest(num);
            if seen.insert(r.clone()) {
                refs.push(r);
            }
        }
        for caps in text_commit_re.captures_iter(text) {
            let hash = caps[1].to_string();
            let r = FollowsRef::Commit(hash);
            if seen.insert(r.clone()) {
                refs.push(r);
            }
        }
        for caps in link_pr_re.captures_iter(text) {
            let num: u32 = caps[1].parse().unwrap();
            let r = FollowsRef::PullRequest(num);
            if seen.insert(r.clone()) {
                refs.push(r);
            }
        }
        for caps in link_commit_re.captures_iter(text) {
            let hash = caps[1].to_string();
            let r = FollowsRef::Commit(hash);
            if seen.insert(r.clone()) {
                refs.push(r);
            }
        }
    };

    if let Some(body) = pr_body {
        parse_text(body);
    }

    for commit in commits {
        parse_text(commit.message());
    }

    refs
}

impl std::hash::Hash for FollowsRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            FollowsRef::PullRequest(n) => {
                0u8.hash(state);
                n.hash(state);
            }
            FollowsRef::Commit(s) => {
                1u8.hash(state);
                s.hash(state);
            }
        }
    }
}

impl Eq for FollowsRef {}

async fn resolve_follows(
    refs: &[FollowsRef],
    session: &dyn Session,
    owner: &str,
    repo: &str,
) -> Result<Vec<ResolvedFollow>> {
    let mut resolved = Vec::new();
    for follow_ref in refs {
        match follow_ref {
            FollowsRef::PullRequest(number) => {
                let pr = session.get_pull_request(owner, repo, *number).await?;
                if !pr.is_merged() {
                    return Err(anyhow!(
                        "Followed PR #{} is not merged.",
                        number
                    ));
                }
                let sha = pr.merge_commit_sha.ok_or_else(|| {
                    anyhow!("Followed PR #{} has no merge commit.", number)
                })?;
                resolved.push(ResolvedFollow {
                    commit_sha: sha,
                    pr_number: Some(*number),
                    orig_base_branch: Some(pr.base.ref_name.clone()),
                });
            }
            FollowsRef::Commit(hash) => {
                resolved.push(ResolvedFollow {
                    commit_sha: hash.clone(),
                    pr_number: None,
                    orig_base_branch: None,
                });
            }
        }
    }
    Ok(resolved)
}

/// Compare whitespace modes and return the "worst" one.
/// Ordering: empty < "ignore-space-change" < "ignore-all-space"
fn worst_whitespace_mode<'a>(a: &'a str, b: &'a str) -> &'a str {
    fn rank(mode: &str) -> u8 {
        match mode {
            "" => 0,
            "ignore-space-change" => 1,
            "ignore-all-space" => 2,
            _ => 0,
        }
    }
    if rank(b) > rank(a) { b } else { a }
}

async fn clone_and_merge_pull_request<'a>(
    github_app: &'a dyn GithubSessionFactory,
    clone_mgr: &'a GitCloneManager,
    req: &'a PRMergeRequest,
    config: Arc<Config>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
) {
    let owner = &req.repo.owner.login();
    let repo = &req.repo.name;

    let session = match github_app.new_session(owner, repo).await {
        Ok(s) => s,
        Err(e) => {
            error!("Error getting new session: {}", e);
            return;
        }
    };
    let held_clone_dir = match clone_mgr.clone(owner, repo).await {
        Ok(h) => h,
        Err(e) => {
            error!("Error getting new session: {}", e);
            return;
        }
    };
    let clone_dir = held_clone_dir.dir();
    let git = Git::new(session.github_host(), session.github_token(), clone_dir);

    merge_pull_request(&git, &session, req, config, slack).await
}

pub async fn merge_pull_request<'a>(
    git: &'a Git,
    session: &'a dyn Session,
    req: &'a PRMergeRequest,
    config: Arc<Config>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
) {
    if let Err(e) = try_merge_pull_request(git, session, req).await {
        let msg = format!(
            "Error backporting PR from {} to {}",
            req.pull_request.head.ref_name, req.target_branch
        );
        error!("{}: {}", msg, e);

        let github_markdown = format!(
            "{}\n<details>\n<summary>Details</summary>\n\n```\n{}\n```\n</details>",
            msg, e
        );
        let slack_markdown = format!("{}\n\n```\n{}\n```", msg, e);

        let attach = SlackAttachmentBuilder::new("")
            .markdown(&slack_markdown)
            .title(
                format!(
                    "Source PR: #{}: \"{}\"",
                    req.pull_request.number, req.pull_request.title
                )
                .as_str(),
            )
            .title_link(req.pull_request.html_url.clone())
            .color("danger")
            .build();

        let messenger = messenger::new(config.clone(), slack.clone());
        messenger.send_to_owner(
            &msg,
            &[attach],
            &req.pull_request.user,
            &req.repo,
            &req.target_branch,
            &req.commits,
        );

        if let Err(e) = session
            .comment_pull_request(
                req.repo.owner.login(),
                &req.repo.name,
                req.pull_request.number,
                &github_markdown,
            )
            .await
        {
            error!(
                "Error making backport failure comment on pull request: {}",
                e
            );
        }

        if let Err(e) = session
            .add_pull_request_labels(
                req.repo.owner.login(),
                &req.repo.name,
                req.pull_request.number,
                vec!["failed-backport".to_string()],
            )
            .await
        {
            error!("Error adding failed-backport label on pull request: {}", e);
        }
    }
}

pub async fn try_merge_pull_request(
    git: &Git,
    session: &dyn Session,
    req: &PRMergeRequest,
) -> Result<github::PullRequest> {
    let pull_request = &req.pull_request;
    if !pull_request.is_merged() {
        return Err(anyhow!(
            "Pull Request #{} is not yet merged.",
            pull_request.number
        ));
    }

    let merge_commit_sha = if let Some(ref sha) = pull_request.merge_commit_sha {
        sha
    } else {
        return Err(anyhow!(
            "Pull Request #{} has no merge commit.",
            pull_request.number
        ));
    };

    // strip everything before last slash
    let regex = Regex::new(r".*/").unwrap();
    let pr_branch_name = format!(
        "{}-{}",
        regex.replace(&pull_request.head.ref_name, ""),
        regex.replace(&req.target_branch, "")
    );

    // make sure there isn't already such a branch
    if git.has_remote_branch(&pr_branch_name)? {
        return Err(anyhow!(
            "PR branch already exists on origin: '{}'",
            pr_branch_name
        ));
    }

    let follows_refs = parse_follows_refs(
        &req.labels,
        req.pull_request.body.as_deref(),
        &req.commits,
        &req.repo.full_name,
    );

    let resolved_follows = if !follows_refs.is_empty() {
        let owner = req.repo.owner.login();
        let repo_name = &req.repo.name;
        resolve_follows(&follows_refs, session, owner, repo_name).await?
    } else {
        vec![]
    };

    setup_cherry_pick_branch(git, &pr_branch_name, &req.target_branch)?;

    let mut overall_whitespace_mode = String::new();

    for follow in &resolved_follows {
        let remote_target = format!("origin/{}", req.target_branch);
        match git.does_branch_contain(&follow.commit_sha, &remote_target) {
            Ok(true) => {
                info!(
                    "Followed commit {} is already on target branch {}, skipping",
                    follow.commit_sha, req.target_branch
                );
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                info!(
                    "Could not check if commit {} is on branch {}: {}, proceeding with cherry-pick",
                    follow.commit_sha, req.target_branch, e
                );
            }
        }

        let orig_base = follow
            .orig_base_branch
            .as_deref()
            .unwrap_or(&pull_request.base.ref_name);

        let (_title, _body, ws_mode) = cherry_pick_single(
            git,
            &follow.commit_sha,
            follow.pr_number,
            &req.target_branch,
            orig_base,
            &req.release_branch_prefix,
        )?;

        overall_whitespace_mode =
            worst_whitespace_mode(&overall_whitespace_mode, &ws_mode).to_string();
    }

    let (title, body, ws_mode) = cherry_pick_single(
        git,
        merge_commit_sha,
        Some(pull_request.number),
        &req.target_branch,
        &pull_request.base.ref_name,
        &req.release_branch_prefix,
    )?;

    let whitespace_mode =
        worst_whitespace_mode(&overall_whitespace_mode, &ws_mode).to_string();

    git.run(&["push", "origin", &format!("HEAD:{}", pr_branch_name)])?;

    let owner = &req.repo.owner.login();
    let repo = &req.repo.name;

    let new_pr = create_pr_with_retry(
        session,
        owner,
        repo,
        &title,
        &body,
        &pr_branch_name,
        &req.target_branch,
    )
    .await?;

    let mut assignees: Vec<String> = pull_request
        .assignees
        .iter()
        .map(|a| a.login().to_string())
        .collect();

    // For new PRs, visibility for the original author suffers because
    // the original author is not a reviewer nor attached to the new PR
    // in any way.  To raise the visibility, add the original PR author
    // to the list of assignees
    if !pull_request.user.login().is_empty()
        && !assignees.contains(&pull_request.user.login().to_string())
    {
        assignees.push(pull_request.user.login().to_string());
    }

    if !assignees.is_empty() {
        session
            .assign_pull_request(owner, repo, new_pr.number, assignees)
            .await?;
    }

    let mut reviewers: Vec<String> = pull_request
        .all_reviewers()
        .into_iter()
        .map(|a| a.login().to_string())
        .collect();
    reviewers.retain(|r| r != pull_request.user.login());
    if !reviewers.is_empty() {
        session
            .request_review(owner, repo, new_pr.number, reviewers)
            .await?;
    }

    if !whitespace_mode.is_empty() {
        let msg = format!(
            "Cherry-pick required option `{}`. Please verify correctness.",
            whitespace_mode
        );
        if let Err(e) = session
            .comment_pull_request(owner, repo, new_pr.number, &msg)
            .await
        {
            error!("Error making whitespace comment on pull request: {}", e);
        }
    }

    Ok(new_pr)
}

fn setup_cherry_pick_branch(git: &Git, pr_branch_name: &str, target_branch: &str) -> Result<()> {
    git.checkout_branch(pr_branch_name, &format!("origin/{}", target_branch))?;
    Ok(())
}

fn cherry_pick_single(
    git: &Git,
    commit_hash: &str,
    pr_number: Option<u32>,
    target_branch: &str,
    orig_base_branch: &str,
    release_branch_prefix: &str,
) -> Result<(String, String, String)> {
    let (user, email) = git.get_commit_author(commit_hash)?;
    let email = format!("user.email={}", email);
    let user = format!("user.name={}", user);
    let user_opts = ["-c", &email, "-c", &user];

    // cherry-pick!

    let mut whitespace_mode = "";
    if let Err(e) = do_cherry_pick(git, commit_hash, &[], &user_opts) {
        info!(
            "Could not cherry-pick normally. Ignoring changed whitespace. {}",
            e
        );

        whitespace_mode = "ignore-space-change";
        if let Err(e) = do_cherry_pick(git, commit_hash, &["-X", whitespace_mode], &user_opts) {
            info!(
                "Could not cherry-pick with `-X {}`. Ignoring all whitespace. {}",
                whitespace_mode, e
            );

            whitespace_mode = "ignore-all-space";
            if let Err(e) = do_cherry_pick(git, commit_hash, &["-X", whitespace_mode], &user_opts) {
                info!("Could not cherry-pick with `-X {}`: {}", whitespace_mode, e);
                return Err(e);
            }
        }
    }

    let desc = git.get_commit_desc(commit_hash)?;
    let (title, body) = make_merge_desc(
        desc,
        commit_hash,
        pr_number,
        target_branch,
        orig_base_branch,
        release_branch_prefix,
    );

    // change commit message
    let mut amend_args = vec![];
    amend_args.extend(user_opts.iter());
    amend_args.extend(["commit", "--amend", "-F", "-"].iter());
    git.run_with_stdin(&amend_args, &format!("{}\n\n{}", &title, &body))?;

    Ok((title, body, whitespace_mode.into()))
}

pub fn cherry_pick(
    git: &Git,
    commit_hash: &str,
    pr_branch_name: &str,
    pr_number: u32,
    target_branch: &str,
    orig_base_branch: &str,
    release_branch_prefix: &str,
) -> Result<(String, String, String)> {
    setup_cherry_pick_branch(git, pr_branch_name, target_branch)?;
    cherry_pick_single(
        git,
        commit_hash,
        Some(pr_number),
        target_branch,
        orig_base_branch,
        release_branch_prefix,
    )
}

fn is_merge_commit(git: &Git, commit_hash: &str) -> Result<bool> {
    let output = git.run(&["rev-list", "--parents", "-1", commit_hash])?;

fn do_cherry_pick(
    git: &Git,
    commit_hash: &str,
    opts: &[&str],
    user_opts: &[&str],
) -> Result<String> {
    git.run(&["reset", "--hard"])?;

    let merge = is_merge_commit(git, commit_hash).unwrap_or(false);

    let mut args = vec!["-c", "merge.renameLimit=999999"];
    args.extend(user_opts.iter());
    args.extend(["cherry-pick", "--allow-empty"].iter());
    if merge {
        args.extend(["-m", "1"].iter());
    }
    args.extend(opts);
    args.push(commit_hash);

    git.run(&args)
}

fn make_merge_desc(
    orig_desc: (String, String),
    commit_hash: &str,
    pr_number: Option<u32>,
    target_branch: &str,
    orig_base_branch: &str,
    release_branch_prefix: &str,
) -> (String, String) {
    // grab original title and strip out the PR number at the end
    let pr_regex = Regex::new(r"(\s*\(#\d+\))+$").unwrap();
    let prev_merge_regex = Regex::new(r"^([^:]+->[^:]+: )+").unwrap();

    // strip out PR from title
    let orig_title = pr_regex.replace(&orig_desc.0, "");
    // strip out previous merge title prefixes
    let mut orig_title = prev_merge_regex.replace(&orig_title, "").into_owned();

    // strip out conventional commit prefix
    let mut prefix = String::new();
    if let Ok(commit) = Commit::new(&orig_title) {
        prefix = commit.type_().to_owned();
        if let Some(s) = commit.scope() {
            prefix += &format!("({})", s);
        }
        if commit.breaking() {
            prefix += "!";
        }
        prefix += ": ";
        orig_title = commit.description().to_owned();
    }

    // strip out 'release' from the prefix to keep titles shorter
    let mut target_branch = target_branch.to_owned();
    if target_branch.starts_with(release_branch_prefix) {
        target_branch = target_branch.replacen(release_branch_prefix, "", 1);
    }
    let mut orig_base_branch = orig_base_branch.to_owned();
    if orig_base_branch.starts_with(release_branch_prefix) {
        orig_base_branch = orig_base_branch.replacen(release_branch_prefix, "", 1);
    }

    let title = format!(
        "{}{}->{}: {}",
        prefix, orig_base_branch, target_branch, orig_title
    );
    let mut body = orig_desc.1;

    if !body.is_empty() {
        body += "\n\n";
    }
    let cherry_pick_note = match pr_number {
        Some(n) => format!("(cherry-picked from {}, PR #{})", commit_hash, n),
        None => format!("(cherry-picked from {})", commit_hash),
    };
    body += &cherry_pick_note;

    (title, body)
}

async fn create_pr_with_retry(
    session: &dyn Session,
    owner: &str,
    repo: &str,
    title: &str,
    body: &str,
    pr_branch_name: &str,
    target_branch: &str,
) -> Result<github::PullRequest> {
    let make_pr =
        || session.create_pull_request(owner, repo, title, body, pr_branch_name, target_branch);
    match make_pr().await {
        Ok(pr) => Ok(pr),
        Err(e) => {
            info!("retrying create_pull_request after 1s due to error: {e}",);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            make_pr().await
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct PRMergeRequest {
    pub repo: github::Repo,
    pub pull_request: github::PullRequest,
    pub target_branch: String,
    pub release_branch_prefix: String,
    pub commits: Vec<github::Commit>,
    pub labels: Vec<github::Label>,
}

struct Runner {
    config: Arc<Config>,
    github_app: Arc<dyn GithubSessionFactory>,
    clone_mgr: Arc<GitCloneManager>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
    metrics: Arc<Metrics>,
}

pub fn req(
    repo: &github::Repo,
    pull_request: &github::PullRequest,
    target_branch: &str,
    release_branch_prefix: &str,
    commits: &[github::Commit],
    labels: &[github::Label],
) -> PRMergeRequest {
    PRMergeRequest {
        repo: repo.clone(),
        pull_request: pull_request.clone(),
        target_branch: target_branch.to_string(),
        release_branch_prefix: release_branch_prefix.to_string(),
        commits: commits.into(),
        labels: labels.into(),
    }
}

pub fn new_runner(
    config: Arc<Config>,
    github_app: Arc<dyn GithubSessionFactory>,
    clone_mgr: Arc<GitCloneManager>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
    metrics: Arc<Metrics>,
) -> Arc<dyn worker::Runner<PRMergeRequest>> {
    Arc::new(Runner {
        config,
        github_app,
        clone_mgr,
        slack,
        metrics,
    })
}

#[async_trait::async_trait]
impl worker::Runner<PRMergeRequest> for Runner {
    async fn handle(&self, req: PRMergeRequest) {
        let _scoped_count = metrics::scoped_inc(&self.metrics.current_backport_count);
        let _scoped_timer = self.metrics.backport_duration.start_timer();

        clone_and_merge_pull_request(
            self.github_app.borrow(),
            self.clone_mgr.borrow(),
            &req,
            self.config.clone(),
            self.slack.clone(),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_merge_desc() {
        let desc = make_merge_desc(
            (
                String::from("Yay, I made a change (#99)"),
                String::from("here is more data about it"),
            ),
            "abcdef",
            Some(99),
            "release/target_branch",
            "source_branch",
            "release/",
        );

        assert_eq!(desc.0, "source_branch->target_branch: Yay, I made a change");
        assert_eq!(
            desc.1,
            "here is more data about it\n\n(cherry-picked from abcdef, PR #99)"
        );
    }

    #[test]
    fn test_make_merge_desc_no_body() {
        let desc = make_merge_desc(
            (String::from("Yay, I made a change (#99)"), String::from("")),
            "abcdef",
            Some(99),
            "the-release-target_branch",
            "source_branch",
            "the-release-",
        );

        assert_eq!(desc.0, "source_branch->target_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_no_release_branch() {
        let desc = make_merge_desc(
            (String::from("Yay, I made a change (#99)"), String::from("")),
            "abcdef",
            Some(99),
            "other_branch",
            "source_branch",
            "release/",
        );

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_from_release_branch() {
        let desc = make_merge_desc(
            (String::from("Yay, I made a change (#99)"), String::from("")),
            "abcdef",
            Some(99),
            "release-other_branch",
            "release-source_branch",
            "release-",
        );

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_multi1() {
        let desc = make_merge_desc(
            (
                String::from("prev_branch->source_branch: Yay, I made a change (#99)"),
                String::from(""),
            ),
            "abcdef",
            Some(99),
            "other_branch",
            "source_branch",
            "release/",
        );

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_multi2() {
        let desc = make_merge_desc(
            (
                String::from(
                    "prev_branch->source_branch: more_branches->prev_branch: Yay, I made a change (#99)",
                ),
                String::from(""),
            ),
            "abcdef",
            Some(99),
            "other_branch",
            "source_branch",
            "release/",
        );

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_no_pr_number() {
        let desc = make_merge_desc(
            (String::from("Some commit message"), String::from("")),
            "abc1234",
            None,
            "release/1.0",
            "master",
            "release/",
        );

        assert_eq!(desc.0, "master->1.0: Some commit message");
        assert_eq!(desc.1, "(cherry-picked from abc1234)");
    }

    #[test]
    fn test_parse_follows_refs_labels() {
        let labels = vec![
            github::Label::new("follows-pr-199"),
            github::Label::new("follows-commit-abc123f"),
            github::Label::new("backport-1.0"),
            github::Label::new("follows-pr-200"),
        ];
        let refs = parse_follows_refs(&labels, None, &[], "owner/repo");
        assert_eq!(
            refs,
            vec![
                FollowsRef::PullRequest(199),
                FollowsRef::Commit("abc123f".to_string()),
                FollowsRef::PullRequest(200),
            ]
        );
    }

    #[test]
    fn test_parse_follows_refs_text() {
        let body = "This PR follows pr 123 and also follows commit deadbeef";
        let refs = parse_follows_refs(&[], Some(body), &[], "owner/repo");
        assert_eq!(
            refs,
            vec![
                FollowsRef::PullRequest(123),
                FollowsRef::Commit("deadbeef".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_follows_refs_text_flexible_formats() {
        let body = "follows-pr-42\nfollows pr-55\nfollows-pr 77";
        let refs = parse_follows_refs(&[], Some(body), &[], "owner/repo");
        assert_eq!(
            refs,
            vec![
                FollowsRef::PullRequest(42),
                FollowsRef::PullRequest(55),
                FollowsRef::PullRequest(77),
            ]
        );
    }

    #[test]
    fn test_parse_follows_refs_commit_messages() {
        let mut commit = github::Commit::new();
        commit.commit.message = "fix stuff\n\nfollows pr 300".to_string();
        let refs = parse_follows_refs(&[], None, &[commit], "owner/repo");
        assert_eq!(refs, vec![FollowsRef::PullRequest(300)]);
    }

    #[test]
    fn test_parse_follows_refs_links() {
        let body = "follows https://github.com/owner/repo/pull/123\nfollows https://github.com/owner/repo/commit/abc1234";
        let refs = parse_follows_refs(&[], Some(body), &[], "owner/repo");
        assert_eq!(
            refs,
            vec![
                FollowsRef::PullRequest(123),
                FollowsRef::Commit("abc1234".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_follows_refs_links_other_repo_ignored() {
        let body = "follows https://github.com/other/repo/pull/123";
        let refs = parse_follows_refs(&[], Some(body), &[], "owner/repo");
        assert_eq!(refs, vec![]);
    }

    #[test]
    fn test_parse_follows_refs_dedup() {
        let labels = vec![github::Label::new("follows-pr-123")];
        let body = "follows pr 123";
        let refs = parse_follows_refs(&labels, Some(body), &[], "owner/repo");
        assert_eq!(refs, vec![FollowsRef::PullRequest(123)]);
    }

    #[test]
    fn test_parse_follows_refs_case_insensitive() {
        let body = "Follows PR 42\nFOLLOWS COMMIT abc1234";
        let refs = parse_follows_refs(&[], Some(body), &[], "owner/repo");
        assert_eq!(
            refs,
            vec![
                FollowsRef::PullRequest(42),
                FollowsRef::Commit("abc1234".to_string()),
            ]
        );
    }
}
