# Octobot Utilities

This package contains command-line utilities for octobot.

## Available Tools

### jira-test

An interactive CLI tool for testing Jira field operations, particularly useful for testing the new release note fields.

**Usage:**
```bash
cargo build --bin jira-test
./target/debug/jira-test /path/to/config.toml
```

**Features:**
- Test setting/getting release note text fields
- Test setting/getting release note channels fields
- View issue details including custom fields
- Interactive menu-driven interface

**Example Session:**
```
$ ./target/debug/jira-test config.toml
Connecting to Jira at jira.company.com...
Connected successfully!

=== Jira Field Tester ===
1. Test release note text field
2. Test release note channels field
3. Get issue details
4. Exit
Choose an option (1-4): 1

=== Release Note Text Field Test ===
Enter Jira issue key (e.g., PROJ-123): PROJ-456
1. Set release note text
2. Get release note text
Choose action (1-2): 1
Enter release note text: Fixed critical authentication bug
✓ Successfully set release note text
```

### octobot-passwd

Sets admin or metrics passwords for the web UI.

**Usage:**
```bash
octobot-passwd <config-file> <admin-username>
octobot-passwd <config-file> --metrics
```

### ldap-check

Tests LDAP authentication and search functionality.

**Usage:**
```bash
ldap-check <config-file> auth
ldap-check <config-file> search
```

### octobot-ask-pass

Git authentication helper (used internally by octobot).

## Configuration

All tools require a valid `config.toml` file. For jira-test, ensure your config includes:

```toml
[jira]
host = "jira.company.com"
username = "your-username"
password = "your-password"

# Optional: Configure the new fields
release_note_text_field = "customfield_10002"
release_note_channels_field = "customfield_10003"
```