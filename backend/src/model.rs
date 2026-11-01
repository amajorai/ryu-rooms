use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomStatus {
    Idle,
    Running,
    Failed,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParticipantRole {
    Host,
    Guest,
    Viewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMessage {
    pub id: String,
    pub role: MessageRole,
    pub text: String,
    pub participant_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomRun {
    pub run_id: String,
    pub request_id: String,
    pub idempotency_key: Option<String>,
    pub status: RunStatus,
    pub partial_text: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomParticipantSummary {
    pub id: String,
    pub display_name: String,
    pub role: ParticipantRole,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSnapshot {
    pub contract: String,
    pub schema_version: u8,
    pub id: String,
    pub status: RoomStatus,
    pub model_id: String,
    pub engine: String,
    pub messages: Vec<RoomMessage>,
    pub current_run: Option<RoomRun>,
    pub participants: Vec<RoomParticipantSummary>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomEvent {
    pub name: String,
    pub data: serde_json::Value,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RoomSession {
    pub room_id: String,
    pub participant_id: String,
    pub role: ParticipantRole,
    pub display_name: String,
    pub secret: String,
}
