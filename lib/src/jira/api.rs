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

#[derive(Deserialize, PartialEq)]
enum DeploymentType {
    Cloud,
    Server,
}

#[derive(Deserialize)]
struct ServerInfo {
    // Optional: older on-prem jira may not report a deployment type at all.
    #[serde(rename = "deploymentType")]
    deployment_type: Option<DeploymentType>,
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

        // Jira Cloud enforces rate limits; retry when throttled.
        let client = HTTPClient::new_with_headers(&api_base, headers)?.with_retry_rate_limits();
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
            myself.display_name.as_deref().unwrap_or("unknown")
        );

        // Detecting the deployment type must succeed: guessing wrong would break
        // cloud-only API paths for the whole lifetime of the session.
        let server_info = client
            .get::<ServerInfo>("/serverInfo")
            .await
            .map_err(|e| anyhow!("Error getting JIRA server info: {}", e))?;
        let is_cloud = server_info.deployment_type == Some(DeploymentType::Cloud);
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
        #[derive(Deserialize)]
        struct ProjectResp {
            id: String,
        }

        #[derive(Serialize)]
        struct AddVersionReq {
            name: String,
            #[serde(rename = "projectId")]
            project_id: u64,
        }

        // Creating a version by project key is deprecated on Jira Cloud: use the
        // numeric project id.
        let project = self
            .client
            .get::<ProjectResp>(&format!("/project/{}", proj))
            .await
            .map_err(|e| anyhow!("Error looking up project {}: {}", proj, e))?;

        let project_id = project
            .id
            .parse::<u64>()
            .map_err(|e| anyhow!("Invalid id \"{}\" for project {}: {}", project.id, proj, e))?;

        let req = AddVersionReq {
            name: version.into(),
            project_id,
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
        let (field, field_id) = match (
            &self.pending_versions_field,
            &self.pending_versions_field_id,
        ) {
            (Some(field), Some(field_id)) => (field, field_id),
            _ => return Ok(HashMap::new()),
        };

        let jql = format!("(project = \"{}\") and \"{}\" is not EMPTY", project, field);
        let encoded_jql = utf8_percent_encode(&jql, NON_ALPHANUMERIC).to_string();

        let result = if self.is_cloud {
            self.search_pending_versions_cloud(&encoded_jql, field_id)
                .await
        } else {
            self.search_pending_versions_server(&encoded_jql, field_id)
                .await
        };

        result.map_err(|e| anyhow!("Error finding pending versions for project {project}: {e}"))
    }
}

const SEARCH_MAX_RESULTS: usize = 100;
// Backstop for cursor pagination in case a misbehaving server returns fresh
// page tokens forever. Set high enough (100k issues) to never trigger on real
// data: results must be complete, so hitting it is an error, not a truncation.
#[cfg(not(test))]
const SEARCH_MAX_PAGES: usize = 1000;
#[cfg(test)]
const SEARCH_MAX_PAGES: usize = 3;

impl JiraSession {
    async fn search_pending_versions_server(
        &self,
        encoded_jql: &str,
        field_id: &str,
    ) -> Result<HashMap<String, Vec<version::Version>>> {
        let mut result: HashMap<String, Vec<version::Version>> = HashMap::new();

        let mut start_at: usize = 0;

        loop {
            let search = self
                .client
                .get::<serde_json::Value>(&format!(
                    "/search?maxResults={}&startAt={}&jql={}",
                    SEARCH_MAX_RESULTS, start_at, encoded_jql
                ))
                .await?;

            let page = parse_pending_versions(&search, field_id);
            let page_len = search["issues"].as_array().map(|a| a.len()).unwrap_or(0);
            result.extend(page);

            let total = search["total"].as_u64().unwrap_or(0) as usize;
            start_at += page_len;

            if page_len == 0 || start_at >= total {
                break;
            }
        }

        Ok(result)
    }

    // Jira Cloud only supports the /search/jql endpoint, which uses cursor-based
    // pagination and returns only explicitly requested fields; on-prem jira only
    // supports /search.
    async fn search_pending_versions_cloud(
        &self,
        encoded_jql: &str,
        field_id: &str,
    ) -> Result<HashMap<String, Vec<version::Version>>> {
        let mut result: HashMap<String, Vec<version::Version>> = HashMap::new();

        let mut next_page_token: Option<String> = None;

        for _ in 0..SEARCH_MAX_PAGES {
            // The issue key must be requested explicitly: /search/jql returns
            // only the fields asked for.
            let mut url = format!(
                "/search/jql?maxResults={}&fields=key,{}&jql={}",
                SEARCH_MAX_RESULTS, field_id, encoded_jql
            );
            if let Some(ref token) = next_page_token {
                url += &format!(
                    "&nextPageToken={}",
                    utf8_percent_encode(token, NON_ALPHANUMERIC)
                );
            }

            let search = self.client.get::<serde_json::Value>(&url).await?;

            result.extend(parse_pending_versions(&search, field_id));

            // A missing/null nextPageToken indicates the last page.
            let new_token = search["nextPageToken"].as_str().map(|s| s.to_string());
            if new_token.is_none() {
                return Ok(result);
            }
            if new_token == next_page_token {
                return Err(anyhow!("Jira search repeated page token {new_token:?}"));
            }
            next_page_token = new_token;
        }

        Err(anyhow!(
            "Jira search did not terminate after {SEARCH_MAX_PAGES} pages"
        ))
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
            .with_body(
                r#"[{"id": "fixVersions", "name": "Fix Version/s"},
                    {"id": "customfield_100", "name": "Pending Versions"}]"#,
            )
            .create_async()
            .await
    }

    async fn new_test_session(server: &mut mockito::Server, deployment_type: &str) -> JiraSession {
        server
            .mock("GET", "/rest/api/2/myself")
            .with_body(r#"{"displayName": "Octo Bot"}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/rest/api/2/serverInfo")
            .with_body(format!(r#"{{"deploymentType": "{}"}}"#, deployment_type))
            .create_async()
            .await;
        mock_fields(server).await;

        let mut config = test_jira_config(&server.url(), JiraAuth::Token("the-token".into()));
        config.pending_versions_field = Some("Pending Versions".into());
        JiraSession::new(&config, None).await.unwrap()
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

    #[tokio::test]
    async fn test_new_session_server_info_error() {
        let mut server = mockito::Server::new_async().await;

        let myself = server
            .mock("GET", "/rest/api/2/myself")
            .with_body(r#"{"displayName": "Octo Bot"}"#)
            .expect(1)
            .create_async()
            .await;
        let server_info = server
            .mock("GET", "/rest/api/2/serverInfo")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;

        let config = test_jira_config(&server.url(), JiraAuth::Token("the-token".into()));
        let err = match JiraSession::new(&config, None).await {
            Ok(_) => panic!("expected server info error"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("Error getting JIRA server info"),
            "unexpected error: {}",
            err
        );

        myself.assert_async().await;
        server_info.assert_async().await;
    }

    #[tokio::test]
    async fn test_new_session_unrecognized_deployment_type() {
        let mut server = mockito::Server::new_async().await;

        let myself = server
            .mock("GET", "/rest/api/2/myself")
            .with_body(r#"{"displayName": "Octo Bot"}"#)
            .expect(1)
            .create_async()
            .await;
        let server_info = server
            .mock("GET", "/rest/api/2/serverInfo")
            .with_body(r#"{"deploymentType": "Mainframe"}"#)
            .expect(1)
            .create_async()
            .await;

        let config = test_jira_config(&server.url(), JiraAuth::Token("the-token".into()));
        let err = match JiraSession::new(&config, None).await {
            Ok(_) => panic!("expected deployment type error"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("Error getting JIRA server info"),
            "unexpected error: {}",
            err
        );

        myself.assert_async().await;
        server_info.assert_async().await;
    }

    #[tokio::test]
    async fn test_add_version_uses_project_id() {
        let mut server = mockito::Server::new_async().await;
        let session = new_test_session(&mut server, "Cloud").await;

        let project = server
            .mock("GET", "/rest/api/2/project/PRJ")
            .with_body(r#"{"id": "10500", "key": "PRJ"}"#)
            .expect(1)
            .create_async()
            .await;

        let create = server
            .mock("POST", "/rest/api/2/version")
            .match_body(mockito::Matcher::Json(json!({
                "name": "1.2.3",
                "projectId": 10500
            })))
            .with_body(r#"{"self": "http://jira/version/400", "id": "400", "name": "1.2.3"}"#)
            .expect(1)
            .create_async()
            .await;

        let version = session.add_version("PRJ", "1.2.3").await.unwrap();
        assert_eq!("400", version.id);
        assert_eq!("1.2.3", version.name);

        project.assert_async().await;
        create.assert_async().await;
    }

    #[tokio::test]
    async fn test_find_pending_versions_server() {
        let mut server = mockito::Server::new_async().await;
        let session = new_test_session(&mut server, "Server").await;

        let jql = r#"(project = "PRJ") and "Pending Versions" is not EMPTY"#;

        let page1 = server
            .mock("GET", "/rest/api/2/search")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("jql".into(), jql.into()),
                mockito::Matcher::UrlEncoded("startAt".into(), "0".into()),
            ]))
            .with_body(
                r#"{"total": 3, "issues": [
                    {"key": "PRJ-1", "fields": {"customfield_100": "1.2, 3.4"}},
                    {"key": "PRJ-2", "fields": {"customfield_100": "5.6"}}
                ]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let page2 = server
            .mock("GET", "/rest/api/2/search")
            .match_query(mockito::Matcher::UrlEncoded("startAt".into(), "2".into()))
            .with_body(
                r#"{"total": 3, "issues": [
                    {"key": "PRJ-3", "fields": {"customfield_100": "7.8"}}
                ]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let expected = hashmap! {
            "PRJ-1".to_string() => vec![
                version::Version::parse("1.2").unwrap(),
                version::Version::parse("3.4").unwrap(),
            ],
            "PRJ-2".to_string() => vec![version::Version::parse("5.6").unwrap()],
            "PRJ-3".to_string() => vec![version::Version::parse("7.8").unwrap()],
        };
        assert_eq!(
            expected,
            session.find_pending_versions("PRJ").await.unwrap()
        );

        page1.assert_async().await;
        page2.assert_async().await;
    }

    #[tokio::test]
    async fn test_find_pending_versions_cloud() {
        let mut server = mockito::Server::new_async().await;
        let session = new_test_session(&mut server, "Cloud").await;

        let jql = r#"(project = "PRJ") and "Pending Versions" is not EMPTY"#;

        let page1 = server
            .mock("GET", "/rest/api/2/search/jql")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("jql".into(), jql.into()),
                mockito::Matcher::UrlEncoded("fields".into(), "key,customfield_100".into()),
            ]))
            .match_request(|req| !req.path_and_query().contains("nextPageToken"))
            .with_body(
                r#"{"nextPageToken": "tok+2", "issues": [
                    {"key": "PRJ-1", "fields": {"customfield_100": "1.2, 3.4"}}
                ]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let page2 = server
            .mock("GET", "/rest/api/2/search/jql")
            .match_query(mockito::Matcher::UrlEncoded(
                "nextPageToken".into(),
                "tok+2".into(),
            ))
            .with_body(
                r#"{"issues": [
                    {"key": "PRJ-2", "fields": {"customfield_100": "5.6"}}
                ]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let expected = hashmap! {
            "PRJ-1".to_string() => vec![
                version::Version::parse("1.2").unwrap(),
                version::Version::parse("3.4").unwrap(),
            ],
            "PRJ-2".to_string() => vec![version::Version::parse("5.6").unwrap()],
        };
        assert_eq!(
            expected,
            session.find_pending_versions("PRJ").await.unwrap()
        );

        page1.assert_async().await;
        page2.assert_async().await;
    }

    #[tokio::test]
    async fn test_find_pending_versions_cloud_single_page() {
        let mut server = mockito::Server::new_async().await;
        let session = new_test_session(&mut server, "Cloud").await;

        // no nextPageToken: a single page is also the last page
        let page = server
            .mock("GET", "/rest/api/2/search/jql")
            .match_query(mockito::Matcher::Any)
            .with_body(
                r#"{"issues": [
                    {"key": "PRJ-1", "fields": {"customfield_100": "1.2"}}
                ]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let expected = hashmap! {
            "PRJ-1".to_string() => vec![version::Version::parse("1.2").unwrap()],
        };
        assert_eq!(
            expected,
            session.find_pending_versions("PRJ").await.unwrap()
        );

        page.assert_async().await;
    }

    #[tokio::test]
    async fn test_find_pending_versions_cloud_repeated_token() {
        let mut server = mockito::Server::new_async().await;
        let session = new_test_session(&mut server, "Cloud").await;

        let page1 = server
            .mock("GET", "/rest/api/2/search/jql")
            .match_query(mockito::Matcher::Any)
            .match_request(|req| !req.path_and_query().contains("nextPageToken"))
            .with_body(
                r#"{"nextPageToken": "tok", "issues": [
                    {"key": "PRJ-1", "fields": {"customfield_100": "1.2"}}
                ]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        // a server stuck returning the same page token must error, not
        // silently return partial results
        let page2 = server
            .mock("GET", "/rest/api/2/search/jql")
            .match_query(mockito::Matcher::UrlEncoded(
                "nextPageToken".into(),
                "tok".into(),
            ))
            .with_body(r#"{"nextPageToken": "tok", "issues": []}"#)
            .expect(1)
            .create_async()
            .await;

        let err = match session.find_pending_versions("PRJ").await {
            Ok(_) => panic!("expected repeated page token error"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("repeated page token"),
            "unexpected error: {}",
            err
        );

        page1.assert_async().await;
        page2.assert_async().await;
    }

    #[tokio::test]
    async fn test_find_pending_versions_cloud_too_many_pages() {
        let mut server = mockito::Server::new_async().await;
        let session = new_test_session(&mut server, "Cloud").await;

        let page1 = server
            .mock("GET", "/rest/api/2/search/jql")
            .match_query(mockito::Matcher::Any)
            .match_request(|req| !req.path_and_query().contains("nextPageToken"))
            .with_body(
                r#"{"nextPageToken": "t1", "issues": [
                    {"key": "PRJ-1", "fields": {"customfield_100": "1.2"}}
                ]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let page2 = server
            .mock("GET", "/rest/api/2/search/jql")
            .match_query(mockito::Matcher::UrlEncoded(
                "nextPageToken".into(),
                "t1".into(),
            ))
            .with_body(r#"{"nextPageToken": "t2", "issues": []}"#)
            .expect(1)
            .create_async()
            .await;

        // a server handing out fresh tokens forever must hit the page cap
        // (SEARCH_MAX_PAGES = 3 in tests) and error rather than loop
        let page3 = server
            .mock("GET", "/rest/api/2/search/jql")
            .match_query(mockito::Matcher::UrlEncoded(
                "nextPageToken".into(),
                "t2".into(),
            ))
            .with_body(r#"{"nextPageToken": "t3", "issues": []}"#)
            .expect(1)
            .create_async()
            .await;

        let err = match session.find_pending_versions("PRJ").await {
            Ok(_) => panic!("expected too many pages error"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("did not terminate"),
            "unexpected error: {}",
            err
        );

        page1.assert_async().await;
        page2.assert_async().await;
        page3.assert_async().await;
    }

    #[tokio::test]
    async fn test_find_pending_versions_no_field_configured() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/rest/api/2/myself")
            .with_body(r#"{"displayName": "Octo Bot"}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/rest/api/2/serverInfo")
            .with_body(r#"{"deploymentType": "Cloud"}"#)
            .create_async()
            .await;
        mock_fields(&mut server).await;

        // no search requests expected at all
        let search = server
            .mock("GET", mockito::Matcher::Regex("/search".into()))
            .expect(0)
            .create_async()
            .await;

        let config = test_jira_config(&server.url(), JiraAuth::Token("the-token".into()));
        let session = JiraSession::new(&config, None).await.unwrap();

        assert!(
            session
                .find_pending_versions("PRJ")
                .await
                .unwrap()
                .is_empty()
        );
        search.assert_async().await;
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
