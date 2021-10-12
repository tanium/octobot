use std::fmt;

use serde_derive::{Deserialize, Serialize};

use serde;
use serde::de::{self, Deserialize as Deserialize2, Deserializer, Visitor};
use serde::ser::{Serialize as Serialize2, Serializer};

use crate::github::models;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CheckSuite {
    pub id: u32,
    pub url: String,
    pub repository: models::Repo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<Conclusion>,
    pub status: CheckStatus,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CheckRun {
    pub name: String,
    pub head_sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_url: Option<String>,
    pub status: CheckStatus,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<Conclusion>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<CheckOutput>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<CheckAction>>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CheckOutput {
    pub title: Option<String>,
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<CheckAnnotation>>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CheckAnnotation {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub annotation_level: String,
    pub message: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CheckAction {}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CheckRunList {
    pub total_count: u32,
    pub check_runs: Vec<CheckRun>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Conclusion {
    Success,
    Failure,
    Neutral,
    Cancelled,
    TimedOut,
    ActionRequired,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CheckStatus {
    Queued,
    InProgress,
    Completed,
}

impl CheckRun {
    pub fn new(name: &str, pr: &models::PullRequest, url: Option<String>) -> CheckRun {
        CheckRun {
            name: name.into(),
            head_sha: pr.head.sha.clone(),
            status: CheckStatus::InProgress,
            conclusion: None,
            completed_at: None,
            details_url: url,
            output: None,
            actions: None,
        }
    }

    pub fn completed(mut self, conclusion: Conclusion) -> CheckRun {
        self.status = CheckStatus::Completed;
        self.conclusion = Some(conclusion);
        self.completed_at = Some(chrono::Utc::now().to_rfc3339());
        self
    }
}

impl CheckOutput {
    pub fn new(title: &str, summary: &str) -> CheckOutput {
        CheckOutput {
            title: Some(title.to_string()),
            summary: Some(summary.to_string()),
            text: None,
            annotations: None,
        }
    }
}

// Serialization / Deserializtion of enums

impl Serialize2 for Conclusion {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let st = match self {
            Conclusion::Success => "success",
            Conclusion::Failure => "failure",
            Conclusion::Neutral => "neutral",
            Conclusion::Cancelled => "cancelled",
            Conclusion::TimedOut => "timed_out",
            Conclusion::ActionRequired => "action_required",
        };
        serializer.serialize_str(st)
    }
}

impl<'de> Deserialize2<'de> for Conclusion {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Conclusion, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ConclusionVisitor;

        impl<'de> Visitor<'de> for ConclusionVisitor {
            type Value = Conclusion;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Conclusion, E>
            where
                E: de::Error,
            {
                match value {
                    "success" => Ok(Conclusion::Success),
                    "failure" => Ok(Conclusion::Failure),
                    "neutral" => Ok(Conclusion::Neutral),
                    "cancelled" => Ok(Conclusion::Cancelled),
                    "timed_out" => Ok(Conclusion::TimedOut),
                    "action_required" => Ok(Conclusion::ActionRequired),
                    _ => Err(E::custom(format!("unexpected conclusion: '{}'", value))),
                }
            }
        }

        deserializer.deserialize_str(ConclusionVisitor)
    }
}

impl Serialize2 for CheckStatus {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let st = match self {
            CheckStatus::Queued => "queued",
            CheckStatus::InProgress => "in_progress",
            CheckStatus::Completed => "completed",
        };
        serializer.serialize_str(st)
    }
}

impl<'de> Deserialize2<'de> for CheckStatus {
    fn deserialize<D>(deserializer: D) -> std::result::Result<CheckStatus, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StatusVisitor;

        impl<'de> Visitor<'de> for StatusVisitor {
            type Value = CheckStatus;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<CheckStatus, E>
            where
                E: de::Error,
            {
                match value {
                    "queued" => Ok(CheckStatus::Queued),
                    "in_progress" => Ok(CheckStatus::InProgress),
                    "completed" => Ok(CheckStatus::Completed),
                    _ => Err(E::custom(format!("unexpected status: '{}'", value))),
                }
            }
        }

        deserializer.deserialize_str(StatusVisitor)
    }
}
