use std::io::{self, Write};

use octobot_lib::config;
use octobot_lib::jira::api::{JiraSession, Session};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().count() < 2 {
        panic!("Usage: jira-test <config file>");
    }

    let config_file = std::env::args().nth(1).unwrap();
    let config = config::new(config_file.into()).expect("Error parsing config");
    let jira_config = config.jira.expect("No JIRA config found");

    println!("Connecting to Jira at {}...", jira_config.host);
    let jira = JiraSession::new(&jira_config, None).await?;
    println!("Connected successfully!\n");

    loop {
        println!("=== Jira Field Tester ===");
        println!("1. Test release note text field");
        println!("2. Test release note channels field");
        println!("3. Test release note status field");
        println!("4. Get issue details");
        println!("5. Exit");
        print!("Choose an option (1-5): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim() {
            "1" => test_release_note_text(&jira).await?,
            "2" => test_release_note_channels(&jira).await?,
            "3" => test_release_note_status(&jira).await?,
            "4" => get_issue_details(&jira).await?,
            "5" => {
                println!("Goodbye!");
                break;
            }
            _ => println!("Invalid option. Please choose 1-5.\n"),
        }
    }

    Ok(())
}

async fn test_release_note_text(jira: &JiraSession) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Release Note Text Field Test ===");

    let issue_key = read_input("Enter Jira issue key (e.g., PROJ-123): ")?;

    println!("1. Set release note text");
    println!("2. Get release note text");
    print!("Choose action (1-2): ");
    io::stdout().flush()?;

    let mut action = String::new();
    io::stdin().read_line(&mut action)?;

    match action.trim() {
        "1" => {
            let text = read_input("Enter release note text: ")?;
            match jira.set_release_note_text(&issue_key, &text).await {
                Ok(()) => println!("✓ Successfully set release note text"),
                Err(e) => println!("✗ Error setting release note text: {}", e),
            }
        }
        "2" => {
            match jira.get_release_note_text(&issue_key).await {
                Ok(Some(text)) => println!("📝 Release note text: {}", text),
                Ok(None) => println!("📝 Release note text: (empty)"),
                Err(e) => println!("✗ Error getting release note text: {}", e),
            }
        }
        _ => println!("Invalid action"),
    }

    println!();
    Ok(())
}

async fn test_release_note_channels(jira: &JiraSession) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Release Note Channels Field Test ===");

    let issue_key = read_input("Enter Jira issue key (e.g., PROJ-123): ")?;

    println!("1. Set release note channels");
    println!("2. Get release note channels");
    print!("Choose action (1-2): ");
    io::stdout().flush()?;

    let mut action = String::new();
    io::stdin().read_line(&mut action)?;

    match action.trim() {
        "1" => {
            let channels = read_input("Enter release note channels (e.g., #releases,#engineering): ")?;
            match jira.set_release_note_channels(&issue_key, &channels).await {
                Ok(()) => println!("✓ Successfully set release note channels"),
                Err(e) => println!("✗ Error setting release note channels: {}", e),
            }
        }
        "2" => {
            match jira.get_release_note_channels(&issue_key).await {
                Ok(Some(channels)) => println!("📢 Release note channels: {}", channels),
                Ok(None) => println!("📢 Release note channels: (empty)"),
                Err(e) => println!("✗ Error getting release note channels: {}", e),
            }
        }
        _ => println!("Invalid action"),
    }

    println!();
    Ok(())
}

async fn test_release_note_status(jira: &JiraSession) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Release Note Status Field Test ===");

    let issue_key = read_input("Enter Jira issue key (e.g., PROJ-123): ")?;

    println!("1. Set release note status");
    println!("2. Get release note status");
    print!("Choose action (1-2): ");
    io::stdout().flush()?;

    let mut action = String::new();
    io::stdin().read_line(&mut action)?;

    match action.trim() {
        "1" => {
            println!("Valid status values:");
            println!("  - None");
            println!("  - Incomplete");
            println!("  - Complete");
            println!("  - Release Note Not Needed");
            let status = read_input("Enter release note status: ")?;
            match jira.set_release_note_status(&issue_key, &status).await {
                Ok(()) => println!("✓ Successfully set release note status"),
                Err(e) => println!("✗ Error setting release note status: {}", e),
            }
        }
        "2" => {
            match jira.get_release_note_status(&issue_key).await {
                Ok(Some(status)) => println!("🔄 Release note status: {}", status),
                Ok(None) => println!("🔄 Release note status: (empty)"),
                Err(e) => println!("✗ Error getting release note status: {}", e),
            }
        }
        _ => println!("Invalid action"),
    }

    println!();
    Ok(())
}

async fn get_issue_details(jira: &JiraSession) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Issue Details ===");

    let issue_key = read_input("Enter Jira issue key (e.g., PROJ-123): ")?;

    match jira.get_issue(&issue_key).await {
        Ok(issue) => {
            println!("🎫 Issue: {}", issue.key);
            if let Some(status) = issue.status {
                println!("📊 Status: {}", status.name);
            } else {
                println!("📊 Status: (unknown)");
            }

            // Test all three new fields
            match jira.get_release_note_text(&issue_key).await {
                Ok(Some(text)) => println!("📝 Release note text: {}", text),
                Ok(None) => println!("📝 Release note text: (empty)"),
                Err(e) => println!("✗ Error getting release note text: {}", e),
            }

            match jira.get_release_note_channels(&issue_key).await {
                Ok(Some(channels)) => println!("📢 Release note channels: {}", channels),
                Ok(None) => println!("📢 Release note channels: (empty)"),
                Err(e) => println!("✗ Error getting release note channels: {}", e),
            }

            match jira.get_release_note_status(&issue_key).await {
                Ok(Some(status)) => println!("🔄 Release note status: {}", status),
                Ok(None) => println!("🔄 Release note status: (empty)"),
                Err(e) => println!("✗ Error getting release note status: {}", e),
            }
        }
        Err(e) => println!("✗ Error getting issue: {}", e),
    }

    println!();
    Ok(())
}

fn read_input(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    print!("{}", prompt);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}