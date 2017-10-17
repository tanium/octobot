use std::collections::HashMap;
use base64;
use http_client::HTTPClient;
use regex::Regex;
use serde_json;
use url::percent_encoding::{utf8_percent_encode, DEFAULT_ENCODE_SET};

use config::JiraConfig;
use jira::models::*;
use version;

pub trait Session : Send + Sync {
    fn get_transitions(&self, key: &str) -> Result<Vec<Transition>, String>;

    fn transition_issue(&self, key: &str, transition: &TransitionRequest) -> Result<(), String>;

    fn comment_issue(&self, key: &str, comment: &str) -> Result<(), String>;

    fn add_version(&self, proj: &str, version: &str) -> Result<(), String>;
    fn get_versions(&self, proj: &str) -> Result<Vec<Version>, String>;
    fn assign_fix_version(&self, key: &str, version: &str) -> Result<(), String>;
    fn reorder_version(&self, version: &Version, position: JiraVersionPosition) -> Result<(), String>;

    fn add_pending_version(&self, key: &str, version: &str) -> Result<(), String>;
    fn remove_pending_versions(&self, key: &str, versions: &Vec<version::Version>) -> Result<(), String>;
    fn find_pending_versions(&self, proj: &str) -> Result<HashMap<String, Vec<version::Version>>, String>;
}

#[derive(Debug)]
pub enum JiraVersionPosition {
    First,
    After(Version),
}

pub struct JiraSession {
    pub client: HTTPClient,
    fix_versions_field: String,
    pending_versions_field: Option<String>,
    pending_versions_field_id: Option<String>,
}

#[derive(Deserialize)]
struct AuthResp {
    pub name: String,
}

// TODO: would be nice to specialize for () return type...
#[derive(Deserialize)]
struct VoidResp {
    pub fix_json_parse: Option<String>,
}


fn lookup_field(field: &str, fields: &Vec<Field>) -> Result<String, String> {
    fields.iter().find(|f| field == f.id || field == f.name)
        .map(|f| f.id.clone())
        .ok_or(format!("Error: Invalid JIRA field: {}", field))
}

impl JiraSession {
    pub fn new(config: &JiraConfig) -> Result<JiraSession, String> {
        let jira_base = config.base_url();
        let api_base = format!("{}/rest/api/2", jira_base);

        let auth = base64::encode(format!("{}:{}", config.username, config.password).as_bytes());
        let client = HTTPClient::new(&api_base)
            .with_headers(hashmap!{
                "Accept" => "application/json".to_string(),
                "Content-Type" => "application/json".to_string(),
                "Authorization" => format!("Basic {}", auth),
            });

        match client.get::<AuthResp>(&format!("{}/rest/auth/1/session", jira_base)) {
            Ok(a) => info!("Logged into JIRA as {}", a.name),
            Err(e) => return Err(format!("Error authenticating to JIRA: {}", e)),
        };

        let fields = try!(client.get::<Vec<Field>>("/field"));

        let pending_versions_field_id = match config.pending_versions_field {
            Some(ref f) => Some(try!(lookup_field(f, &fields))),
            None => None,
        };
        let fix_versions_field = try!(lookup_field(&config.fix_versions(), &fields));

        debug!("Pending Version field: {:?}", pending_versions_field_id);
        debug!("Fix Versions field: {:?}", fix_versions_field);

        Ok(JiraSession{
            client: client,
            fix_versions_field: fix_versions_field,
            pending_versions_field: config.pending_versions_field.clone(),
            pending_versions_field_id: pending_versions_field_id,
        })
    }
}

impl Session for JiraSession {
    fn get_transitions(&self, key: &str) -> Result<Vec<Transition>, String> {
        #[derive(Deserialize)]
        struct TransitionsResp {
            transitions: Vec<Transition>,
        }
        let resp: TransitionsResp = try!(self.client.get(&format!("/issue/{}/transitions?expand=transitions.fields", key)));
        Ok(resp.transitions)
    }

    fn transition_issue(&self, key: &str, req: &TransitionRequest) -> Result<(), String> {
        try!(self.client.post::<VoidResp, TransitionRequest>(&format!("/issue/{}/transitions", key), &req));
        Ok(())
    }

    fn comment_issue(&self, key: &str, comment: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct CommentReq {
            body: String,
        }

        let req = CommentReq { body: comment.to_string() };
        try!(self.client.post::<Comment, CommentReq>(&format!("/issue/{}/comment", key), &req));
        Ok(())
    }

    fn add_version(&self, proj: &str, version: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct AddVersionReq {
            name: String,
            project: String,
        }

        let req = AddVersionReq { name: version.into(), project: proj.into() };
        try!(self.client.post::<VoidResp, AddVersionReq>("/version", &req));
        Ok(())
    }

    fn get_versions(&self, proj: &str) -> Result<Vec<Version>, String> {
        self.client.get::<Vec<Version>>(&format!("/project/{}/versions", proj))
    }

    fn assign_fix_version(&self, key: &str, version: &str) -> Result<(), String> {
        let field = self.fix_versions_field.clone();
        let req = json!({
            "update": {
                field: [{"add" : {"name" : version}}]
            }
        });

        try!(self.client.put::<VoidResp, serde_json::Value>(&format!("/issue/{}", key), &req));
        Ok(())
    }

    fn reorder_version(&self, version: &Version, position: JiraVersionPosition) -> Result<(), String> {
        let req = match position {
            JiraVersionPosition::First => {
                json!({
                    "position": "First"
                })
            },
            JiraVersionPosition::After(v) => {
                json!({
                    "after": v.uri
                })
            },
        };

        try!(self.client.post::<VoidResp, serde_json::Value>(&format!("/version/{}/move", version.id), &req));
        Ok(())
    }

    fn add_pending_version(&self, key: &str, version: &str) -> Result<(), String> {
        if let Some(ref field) = self.pending_versions_field_id.clone() {
            let issue = try!(self.client.get::<serde_json::Value>(&format!("/issue/{}", key)));

            let version_parsed = match version::Version::parse(version) {
                Some(v) => v,
                None => return Err(format!("Unable to parse version: {}", version)),
            };

            let mut pending_versions = parse_pending_version_field(&issue["fields"][field]);
            pending_versions.push(version_parsed);

            pending_versions.sort();
            pending_versions.dedup_by(|a, b| a == b);

            let new_value = pending_versions.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ");

            let req = json!({
                "update": {
                    field.to_string(): [{ "set": new_value }]
                }
            });

            try!(self.client.put::<VoidResp, serde_json::Value>(&format!("/issue/{}", key), &req));
        }
        Ok(())
    }


    fn remove_pending_versions(&self, key: &str, versions: &Vec<version::Version>) -> Result<(), String> {
        if let Some(ref field_id) = self.pending_versions_field_id.clone() {
            let issue = try!(self.client.get::<serde_json::Value>(&format!("/issue/{}", key)));

            let pending_versions = parse_pending_version_field(&issue["fields"][field_id]);
            let new_pending_versions = pending_versions.iter()
                .filter(|v| !versions.contains(v))
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ");

            let req = json!({
                "update": {
                    field_id.to_string(): [{ "set": new_pending_versions }]
                }
            });

            try!(self.client.put::<VoidResp, serde_json::Value>(&format!("/issue/{}", key), &req));
        }
        Ok(())
    }

    fn find_pending_versions(&self, project: &str) -> Result<HashMap<String, Vec<version::Version>>, String> {
        if let Some(ref field) = self.pending_versions_field.clone() {
            if let Some(ref field_id) = self.pending_versions_field_id {
                let jql = format!("(project = {}) and \"{}\" is not EMPTY", project, field);
                let search = try!(self.client.get::<serde_json::Value>(
                        &format!("/search?maxResults=5000&jql={}", utf8_percent_encode(&jql, DEFAULT_ENCODE_SET))));
                return Ok(parse_pending_versions(&search, &field_id));
            }
        }

        Ok(HashMap::new())
    }
}

fn parse_pending_version_field(field: &serde_json::Value) -> Vec<version::Version> {
    let re = Regex::new(r"\s*,\s*").unwrap();
    re.split(field.as_str().unwrap_or("").trim())
        .filter_map(|s| version::Version::parse(s))
        .collect::<Vec<_>>()
}

fn parse_pending_versions(search: &serde_json::Value, field_id: &str) -> HashMap<String, Vec<version::Version>> {
    search["issues"].as_array().unwrap_or(&vec![]).into_iter().filter_map(|issue| {
        let key  = issue["key"].as_str().unwrap_or("").to_string();
        let list = parse_pending_version_field(&issue["fields"][field_id]);
        if key.is_empty() || list.is_empty() {
            None
        } else {
            Some((key, list))
        }
    })
    .collect::<HashMap<_, _>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pending_versions() {
        let search = json!({
            "issues": [
                {
                    "key": "KEY-1",
                    "fields": {}
                },
                {
                    "key": "KEY-2",
                    "fields": {
                        "the-field": "  1.2, 3.4,5,7.7.7  "
                    }
                },
                {
                    "key": "KEY-3",
                    "fields": {
                        "the-field": "1.2,  "
                    }
                }
            ]
        });
        let expected = hashmap! {
            "KEY-2".to_string() => vec![
                version::Version::parse("1.2").unwrap(),
                version::Version::parse("3.4").unwrap(),
                version::Version::parse("5").unwrap(),
                version::Version::parse("7.7.7").unwrap()
            ],
            "KEY-3".to_string() => vec![
                version::Version::parse("1.2").unwrap(),
            ],
        };

        let versions = parse_pending_versions(&search, "the-field");
        assert_eq!(expected, versions);
    }
}
