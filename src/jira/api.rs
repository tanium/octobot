use base64;
use http_client::HTTPClient;
use serde_json;

use config::JiraConfig;
use jira::models::*;

pub trait Session : Send + Sync {
    fn get_transitions(&self, key: &str) -> Result<Vec<Transition>, String>;

    fn transition_issue(&self, key: &str, transition: &TransitionRequest) -> Result<(), String>;

    fn comment_issue(&self, key: &str, comment: &str) -> Result<(), String>;

    fn add_version(&self, proj: &str, version: &str) -> Result<(), String>;
    fn assign_fix_version(&self, key: &str, version: &str) -> Result<(), String>;
    fn add_pending_version(&self, key: &str, version: &str) -> Result<(), String>;
}

pub struct JiraSession {
    pub client: HTTPClient,
    fix_versions_field: String,
    pending_versions_field: Option<String>,
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
        let jira_base;
        if config.host.starts_with("http") {
            jira_base = config.host.clone();
        } else {
            jira_base = format!("https://{}", config.host);
        }

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

        let pending_versions_field = match config.pending_versions_field {
            Some(ref f) => Some(try!(lookup_field(f, &fields))),
            None => None,
        };
        let fix_versions_field = try!(lookup_field(&config.fix_versions(), &fields));

        debug!("Pending Version field: {:?}", pending_versions_field);
        debug!("Fix Versions field: {:?}", fix_versions_field);

        Ok(JiraSession{
            client: client,
            fix_versions_field: fix_versions_field,
            pending_versions_field: pending_versions_field,
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
        // Versions the way we're using them are probably unique anyway, so don't spend the
        // extra work to check if it exists first.
        if let Err(e) = self.client.post::<VoidResp, AddVersionReq>("/version", &req) {
            if e.find("A version with this name already exists in this project").is_none() {
                return Err(e);
            }
        }

        Ok(())
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

    fn add_pending_version(&self, key: &str, version: &str) -> Result<(), String> {
        if let Some(ref field) = self.pending_versions_field.clone() {
            let issue = try!(self.client.get::<serde_json::Value>(&format!("/issue/{}", key)));

            let mut value : String = issue["fields"][field].as_str().unwrap_or("").to_string();
            if value != "" {
                value += ", ";
            }
            value += version;

            let req = json!({
                "update": {
                    field.to_string(): [{ "set": value }]
                }
            });

            try!(self.client.put::<VoidResp, serde_json::Value>(&format!("/issue/{}", key), &req));
        }
        Ok(())
    }
}
