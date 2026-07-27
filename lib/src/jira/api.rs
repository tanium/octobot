use std::collections::HashMap;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use base64::{self, Engine};
use log::{debug, info};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use regex::Regex;
use serde_derive::{Deserialize, Serialize};
use serde_json;
use serde_json::json;

use crate::config::{JiraAuth, JiraConfig};
use crate::errors::*;
use crate::http_client::HTTPClient;
use crate::jira::models::*;
use crate::metrics::Metrics;
use crate::version;

#[async_trait]
pub trait Session: Send + Sync {
    async fn get_issue(&self, key: &str) -> Result<Issue>;
    async fn get_transitions(&self, key: &str) -> Result<Vec<Transition>>;

    async fn transition_issue(&self, key: &str, transition: &TransitionRequest) -> Result<()>;

    async fn comment_issue(&self, key: &str, comment: &str) -> Result<()>;

    async fn add_version(&self, proj: &str, version: &str) -> Result<Version>;
    async fn get_versions(&self, proj: &str) -> Result<Vec<Version>>;
    async fn assign_fix_version(&self, key: &str, version: &str) -> Result<()>;
    async fn reorder_version(&self, version: &Version, position: JiraVersionPosition)
    -> Result<()>;

    async fn add_pending_version(&self, key: &str, version: &str) -> Result<()>;
    async fn remove_pending_versions(&self, key: &str, versions: &[version::Version])
    -> Result<()>;
    async fn find_pending_versions(
        &self,
        proj: &str,
    ) -> Result<HashMap<String, Vec<version::Version>>>;
}

#[derive(Debug)]
pub enum JiraVersionPosition {
    First,
    After(Version),
}

pub struct JiraSession {
    pub client: HTTPClient,
    is_cloud: bool,
    fix_versions_field: String,
    pending_versions_field: Option<String>,
    pending_versions_field_id: Option<String>,
    restrict_comment_visibility_to_role: Option<String>,
}

#[derive(Deserialize)]
struct Myself {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct ServerInfo {
    #[serde(rename = "deploymentType")]
    deployment_type: Option<String>,
}

fn lookup_field(field: &str, fields: &[Field]) -> Result<String> {
    fields
        .iter()
        .find(|f| field == f.id || field == f.name)
        .map(|f| f.id.clone())
        .ok_or_else(|| anyhow!("Error: Invalid JIRA field: {}", field))
}

impl JiraSession {
    pub async fn new(config: &JiraConfig, metrics: Option<Arc<Metrics>>) -> Result<JiraSession> {
        let jira_base = &config.base_url;
        let api_base = format!("{}/rest/api/2", jira_base);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::ACCEPT, "application/json".parse().unwrap());

        // Note: Jira Cloud does not support bearer tokens: use basic auth with an API
        // token as the password instead.
        let auth_header = match &config.auth {
            JiraAuth::Basic { username, password } => {
                let auth = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", username, password));
                format!("Basic {}", auth)
            }
            JiraAuth::Token(token) => format!("Bearer {}", token),
        };
        headers.insert(
            reqwest::header::AUTHORIZATION,
            auth_header
                .parse()
                .map_err(|e| anyhow!("Invalid auth header: {}", e))?,
        );

        let client = HTTPClient::new_with_headers(&api_base, headers)?;
        let client = match metrics {
            None => client,
            Some(ref m) => {
                client.with_metrics(m.jira_api_responses.clone(), m.jira_api_duration.clone())
            }
        };

        // Check that the auth is good: /myself exists on both cloud and on-prem jira.
        let myself = client
            .get::<Myself>("/myself")
            .await
            .map_err(|e| anyhow!("Error authenticating to JIRA: {}", e))?;

        info!(
            "Logged into JIRA as \"{}\"",
            myself.display_name.unwrap_or_default()
        );

        let is_cloud = match client.get::<ServerInfo>("/serverInfo").await {
            Ok(info) => info.deployment_type.as_deref() == Some("Cloud"),
            Err(e) => {
                log::warn!(
                    "Error getting JIRA server info; assuming server deployment: {}",
                    e
                );
                false
            }
        };
        info!(
            "JIRA deployment type: {}",
            if is_cloud { "cloud" } else { "server" }
        );

        let fields = client.get::<Vec<Field>>("/field").await?;

        let pending_versions_field_id = match config.pending_versions_field {
            Some(ref f) => Some(lookup_field(f, &fields)?),
            None => None,
        };
        let fix_versions_field = lookup_field(&config.fix_versions_field, &fields)?;

        debug!("Pending Version field: {:?}", pending_versions_field_id);
        debug!("Fix Versions field: {:?}", fix_versions_field);

        Ok(JiraSession {
            client,
            is_cloud,
            fix_versions_field,
            pending_versions_field: config.pending_versions_field.clone(),
            pending_versions_field_id,
            restrict_comment_visibility_to_role: config.restrict_comment_visibility_to_role.clone(),
        })
    }

    pub fn is_cloud(&self) -> bool {
        self.is_cloud
    }
}

#[async_trait]
impl Session for JiraSession {
    async fn get_issue(&self, key: &str) -> Result<Issue> {
        self.client
            .get::<Issue>(&format!("/issue/{}?fields=status", key))
            .await
            .map_err(|e| anyhow!("Error creating getting issue [{}]: {}", key, e))
    }

    async fn get_transitions(&self, key: &str) -> Result<Vec<Transition>> {
        #[derive(Deserialize)]
        struct TransitionsResp {
            transitions: Vec<Transition>,
        }
        let resp = self
            .client
            .get::<TransitionsResp>(&format!(
                "/issue/{}/transitions?expand=transitions.fields",
                key
            ))
            .await
            .map_err(|e| anyhow!("Error creating getting transitions for [{}]: {}", key, e))?;
        Ok(resp.transitions)
    }

    async fn transition_issue(&self, key: &str, req: &TransitionRequest) -> Result<()> {
        self.client
            .post_void(&format!("/issue/{}/transitions", key), &req)
            .await
            .map_err(|e| anyhow!("Error transitioning [{}]: {}", key, e))
    }

    async fn comment_issue(&self, key: &str, comment: &str) -> Result<()> {
        #[derive(Serialize)]
        struct VisibilityReq {
            #[serde(rename = "type")]
            type_name: String,
            value: String,
        }

        #[derive(Serialize)]
        struct CommentReq {
            body: String,
            visibility: Option<VisibilityReq>,
        }

        let mut req = CommentReq {
            body: comment.to_string(),
            visibility: None,
        };

        if let Some(r) = &self.restrict_comment_visibility_to_role {
            req.visibility = Some(VisibilityReq {
                type_name: "role".to_string(),
                value: r.clone(),
            });

            let result = self
                .client
                .post_void::<CommentReq>(&format!("/issue/{}/comment", key), &req)
                .await;
            if result.is_ok() {
                return Ok(());
            }

            req.visibility = None;
            // Fall-through to making the request without the visibility restriction
        }

        self.client
            .post_void::<CommentReq>(&format!("/issue/{}/comment", key), &req)
            .await
            .map_err(|e| anyhow!("Error commenting on [{}]: {}", key, e))
    }

    async fn add_version(&self, proj: &str, version: &str) -> Result<Version> {
        #[derive(Serialize)]
        struct AddVersionReq {
            name: String,
            project: String,
        }

        let req = AddVersionReq {
            name: version.into(),
            project: proj.into(),
        };
        self.client
            .post::<Version, AddVersionReq>("/version", &req)
            .await
            .map_err(|e| {
                anyhow!(
                    "Error adding version {} to project {}: {}",
                    version,
                    proj,
                    e
                )
            })
    }

    async fn get_versions(&self, proj: &str) -> Result<Vec<Version>> {
        self.client
            .get::<Vec<Version>>(&format!("/project/{}/versions", proj))
            .await
            .map_err(|e| anyhow!("Error getting versions for project {}: {}", proj, e))
    }

    async fn assign_fix_version(&self, key: &str, version: &str) -> Result<()> {
        let field = self.fix_versions_field.clone();
        let req = json!({
            "update": {
                field: [{"add" : {"name" : version}}]
            }
        });

        self.client
            .put_void(&format!("/issue/{}", key), &req)
            .await
            .map_err(|e| anyhow!("Error adding fix-version {} to [{}]: {}", version, key, e))
    }

    async fn reorder_version(
        &self,
        version: &Version,
        position: JiraVersionPosition,
    ) -> Result<()> {
        let req = match position {
            JiraVersionPosition::First => {
                json!({
                    "position": "First"
                })
            }
            JiraVersionPosition::After(v) => {
                json!({
                    "after": v.uri
                })
            }
        };

        self.client
            .post_void(&format!("/version/{}/move", version.id), &req)
            .await
            .map_err(|e| anyhow!("Error reordering version {}: {}", version.name, e))
    }

    async fn add_pending_version(&self, key: &str, version: &str) -> Result<()> {
        if let Some(ref field) = self.pending_versions_field_id.clone() {
            let issue = self
                .client
                .get::<serde_json::Value>(&format!("/issue/{}", key))
                .await?;

            let version_parsed = match version::Version::parse(version) {
                Some(v) => v,
                None => return Err(anyhow!("Unable to parse version: {}", version)),
            };

            let mut pending_versions = parse_pending_version_field(&issue["fields"][field]);
            pending_versions.push(version_parsed);

            pending_versions.sort();
            pending_versions.dedup_by(|a, b| a == b);

            let new_value = pending_versions
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ");

            let req = json!({
                "update": {
                    field.to_string(): [{ "set": new_value }]
                }
            });

            self.client
                .put_void(&format!("/issue/{}", key), &req)
                .await
                .map_err(|e| {
                    anyhow!(
                        "Error adding pending version {} to [{}]: {}",
                        version,
                        key,
                        e
                    )
                })?;
        }
        Ok(())
    }

    async fn remove_pending_versions(
        &self,
        key: &str,
        versions: &[version::Version],
    ) -> Result<()> {
        if let Some(ref field_id) = self.pending_versions_field_id.clone() {
            let issue = self
                .client
                .get::<serde_json::Value>(&format!("/issue/{}", key))
                .await?;

            let pending_versions = parse_pending_version_field(&issue["fields"][field_id]);
            let new_pending_versions = pending_versions
                .iter()
                .filter(|v| !versions.contains(v))
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ");

            let req = json!({
                "update": {
                    field_id.to_string(): [{ "set": new_pending_versions }]
                }
            });

            self.client
                .put_void(&format!("/issue/{}", key), &req)
                .await
                .map_err(|e| {
                    anyhow!(
                        "Error removing pending versions {:?} from [{}]: {}",
                        versions,
                        key,
                        e
                    )
                })?;
        }
        Ok(())
    }

    async fn find_pending_versions(
        &self,
        project: &str,
    ) -> Result<HashMap<String, Vec<version::Version>>> {
        let mut result: HashMap<String, Vec<version::Version>> = HashMap::new();

        if let Some(ref field) = self.pending_versions_field.clone() {
            if let Some(ref field_id) = self.pending_versions_field_id {
                let jql = format!("(project = \"{}\") and \"{}\" is not EMPTY", project, field);
                let encoded_jql = utf8_percent_encode(&jql, NON_ALPHANUMERIC).to_string();

                let mut start_at: usize = 0;
                let max_results: usize = 100;

                loop {
                    let search = self
                        .client
                        .get::<serde_json::Value>(&format!(
                            "/search?maxResults={}&startAt={}&jql={}",
                            max_results, start_at, encoded_jql
                        ))
                        .await
                        .map_err(|e| {
                            anyhow!("Error finding pending versions for project {project}: {e}")
                        })?;

                    let page = parse_pending_versions(&search, field_id);
                    let page_len = search["issues"].as_array().map(|a| a.len()).unwrap_or(0);
                    result.extend(page);

                    let total = search["total"].as_u64().unwrap_or(0) as usize;
                    start_at += page_len;

                    if page_len == 0 || start_at >= total {
                        break;
                    }
                }
            }
        }

        Ok(result)
    }
}

fn parse_pending_version_field(field: &serde_json::Value) -> Vec<version::Version> {
    let re = Regex::new(r"\s*,\s*").unwrap();
    re.split(field.as_str().unwrap_or("").trim())
        .filter_map(version::Version::parse)
        .collect::<Vec<_>>()
}

fn parse_pending_versions(
    search: &serde_json::Value,
    field_id: &str,
) -> HashMap<String, Vec<version::Version>> {
    search["issues"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|issue| {
            let key = issue["key"].as_str().unwrap_or("").to_string();
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
    use maplit::hashmap;

    fn test_jira_config(url: &str, auth: JiraAuth) -> JiraConfig {
        JiraConfig {
            base_url: url.into(),
            auth,
            progress_states: vec![],
            review_states: vec![],
            resolved_states: vec![],
            frozen_states: vec![],
            fixed_resolutions: vec![],
            fix_versions_field: "fixVersions".into(),
            pending_versions_field: None,
            restrict_comment_visibility_to_role: None,
        }
    }

    async fn mock_fields(server: &mut mockito::Server) -> mockito::Mock {
        server
            .mock("GET", "/rest/api/2/field")
            .with_body(r#"[{"id": "fixVersions", "name": "Fix Version/s"}]"#)
            .create_async()
            .await
    }

    #[tokio::test]
    async fn test_new_session_basic_auth_cloud() {
        let mut server = mockito::Server::new_async().await;

        let expected_auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("me@company.com:api-token")
        );

        let myself = server
            .mock("GET", "/rest/api/2/myself")
            .match_header("authorization", expected_auth.as_str())
            .with_body(r#"{"displayName": "Octo Bot"}"#)
            .expect(1)
            .create_async()
            .await;
        let server_info = server
            .mock("GET", "/rest/api/2/serverInfo")
            .with_body(r#"{"deploymentType": "Cloud"}"#)
            .expect(1)
            .create_async()
            .await;
        let fields = mock_fields(&mut server).await;

        let config = test_jira_config(
            &server.url(),
            JiraAuth::Basic {
                username: "me@company.com".into(),
                password: "api-token".into(),
            },
        );
        let session = JiraSession::new(&config, None).await.unwrap();
        assert!(session.is_cloud());

        myself.assert_async().await;
        server_info.assert_async().await;
        fields.assert_async().await;
    }

    #[tokio::test]
    async fn test_new_session_token_auth_server() {
        let mut server = mockito::Server::new_async().await;

        let myself = server
            .mock("GET", "/rest/api/2/myself")
            .match_header("authorization", "Bearer the-token")
            .with_body(r#"{"displayName": "Octo Bot"}"#)
            .expect(1)
            .create_async()
            .await;
        // no deploymentType: older jira servers may not report one
        let server_info = server
            .mock("GET", "/rest/api/2/serverInfo")
            .with_body(r#"{}"#)
            .expect(1)
            .create_async()
            .await;
        let fields = mock_fields(&mut server).await;

        let config = test_jira_config(&server.url(), JiraAuth::Token("the-token".into()));
        let session = JiraSession::new(&config, None).await.unwrap();
        assert!(!session.is_cloud());

        myself.assert_async().await;
        server_info.assert_async().await;
        fields.assert_async().await;
    }

    #[tokio::test]
    async fn test_new_session_bad_auth() {
        let mut server = mockito::Server::new_async().await;

        let myself = server
            .mock("GET", "/rest/api/2/myself")
            .with_status(401)
            .expect(1)
            .create_async()
            .await;

        let config = test_jira_config(&server.url(), JiraAuth::Token("expired".into()));
        let err = match JiraSession::new(&config, None).await {
            Ok(_) => panic!("expected auth error"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("Error authenticating to JIRA"),
            "unexpected error: {}",
            err
        );

        myself.assert_async().await;
    }

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
