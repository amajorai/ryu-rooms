use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    limits::{self, MAX_HISTORY_MESSAGES, MAX_RETAINED_RUNS},
    model::{
        MessageRole, ParticipantRole, RoomMessage, RoomParticipantSummary, RoomRun, RoomSession,
        RoomSnapshot, RoomStatus, RunStatus,
    },
};

const CONTRACT: &str = "rooms/1";
const ENGINE: &str = "mesh-llm";

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error")]
    Database(#[from] rusqlite::Error),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("room not found")]
    NotFound,
    #[error("room is busy")]
    RoomBusy,
    #[error("room is closed")]
    Closed,
    #[error("permission denied")]
    Forbidden,
    #[error("invite unavailable")]
    InviteUnavailable,
    #[error("conflicting request")]
    Conflict,
    #[error("store lock unavailable")]
    Lock,
}

impl StoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Database(_) | Self::Lock => "storeUnavailable",
            Self::Invalid(_) => "invalidArgs",
            Self::NotFound => "roomNotFound",
            Self::RoomBusy => "roomBusy",
            Self::Closed => "roomClosed",
            Self::Forbidden => "forbidden",
            Self::InviteUnavailable => "inviteUnavailable",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateRoomInput {
    pub model_id: String,
    pub share_origin: String,
}

#[derive(Debug, Clone)]
pub struct CreatedRoom {
    pub room: RoomSnapshot,
    pub invite: String,
    pub share_origin: String,
}

#[derive(Debug, Clone)]
pub struct IssuedInvite {
    pub invite: String,
    pub share_origin: String,
}

#[derive(Debug, Clone)]
pub struct SubmitTurnInput {
    pub text: String,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AcceptedRun {
    pub run: RoomRun,
    pub user_message: RoomMessage,
    pub snapshot: RoomSnapshot,
}

#[derive(Debug, Clone)]
pub struct RunTerminal {
    pub status: RunStatus,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone)]
pub struct RoomStore {
    connection: Arc<Mutex<Connection>>,
}

impl RoomStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|_| StoreError::Lock)?;
        }
        let connection = Connection::open(path)?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.initialize()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory()?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.initialize()?;
        Ok(store)
    }

    fn with_connection<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let connection = self.connection.lock().map_err(|_| StoreError::Lock)?;
        f(&connection)
    }

    fn initialize(&self) -> Result<(), StoreError> {
        self.with_connection(|connection| {
            connection.execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS rooms (
                   id TEXT PRIMARY KEY,
                   schema_version INTEGER NOT NULL,
                   contract TEXT NOT NULL,
                   status TEXT NOT NULL,
                   model_id TEXT NOT NULL,
                   engine TEXT NOT NULL,
                   share_origin TEXT NOT NULL,
                   created_at TEXT NOT NULL,
                   updated_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS room_messages (
                   id TEXT PRIMARY KEY,
                   room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
                   role TEXT NOT NULL,
                   text TEXT NOT NULL,
                   participant_id TEXT,
                   created_at TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS room_messages_room_created
                   ON room_messages(room_id, created_at, id);
                 CREATE TABLE IF NOT EXISTS room_runs (
                   id TEXT PRIMARY KEY,
                   room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
                   request_id TEXT NOT NULL,
                   idempotency_key TEXT,
                   input_text TEXT NOT NULL,
                   status TEXT NOT NULL,
                   partial_text TEXT NOT NULL DEFAULT '',
                   error_code TEXT,
                   error_message TEXT,
                   started_at TEXT,
                   finished_at TEXT,
                   created_at TEXT NOT NULL,
                   UNIQUE(room_id, idempotency_key)
                 );
                 CREATE INDEX IF NOT EXISTS room_runs_room_created
                   ON room_runs(room_id, created_at, id);
                 CREATE TABLE IF NOT EXISTS room_invites (
                   room_id TEXT PRIMARY KEY REFERENCES rooms(id) ON DELETE CASCADE,
                   digest BLOB NOT NULL,
                   expires_at TEXT NOT NULL,
                   revoked_at TEXT
                 );
                 CREATE TABLE IF NOT EXISTS room_participants (
                   id TEXT PRIMARY KEY,
                   room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
                   display_name TEXT NOT NULL,
                   role TEXT NOT NULL,
                   online INTEGER NOT NULL,
                   created_at TEXT NOT NULL,
                   last_seen_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS room_sessions (
                   digest BLOB PRIMARY KEY,
                   room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
                   participant_id TEXT NOT NULL REFERENCES room_participants(id) ON DELETE CASCADE,
                   role TEXT NOT NULL,
                   expires_at TEXT NOT NULL,
                   revoked_at TEXT
                 );",
            )?;
            Ok(())
        })
    }

    pub async fn create_room(&self, input: CreateRoomInput) -> Result<CreatedRoom, StoreError> {
        let model_id = limits::validate_model_id(&input.model_id).map_err(StoreError::Invalid)?;
        let share_origin =
            limits::validate_share_origin(&input.share_origin).map_err(StoreError::Invalid)?;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let room_id = opaque_id("room_");
        let participant_id = opaque_id("member_");
        let invite = opaque_secret();
        let invite_digest = digest(&invite);
        let expires_at = (now + Duration::seconds(limits::INVITE_TTL_SECS)).to_rfc3339();

        self.with_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO rooms (id, schema_version, contract, status, model_id, engine, share_origin, created_at, updated_at)
                 VALUES (?1, 1, ?2, 'idle', ?3, ?4, ?5, ?6, ?6)",
                params![room_id, CONTRACT, model_id, ENGINE, share_origin, now_text],
            )?;
            transaction.execute(
                "INSERT INTO room_invites (room_id, digest, expires_at) VALUES (?1, ?2, ?3)",
                params![room_id, invite_digest, expires_at],
            )?;
            transaction.execute(
                "INSERT INTO room_participants (id, room_id, display_name, role, online, created_at, last_seen_at)
                 VALUES (?1, ?2, 'Host', 'host', 1, ?3, ?3)",
                params![participant_id, room_id, now_text],
            )?;
            Ok(())
        })?;

        let room = self.snapshot(&room_id).await?.ok_or(StoreError::NotFound)?;
        Ok(CreatedRoom {
            room,
            invite,
            share_origin,
        })
    }

    pub async fn list_rooms(&self) -> Result<Vec<RoomSnapshot>, StoreError> {
        let ids = self.with_connection(|connection| {
            let mut statement =
                connection.prepare("SELECT id FROM rooms ORDER BY updated_at DESC, id DESC")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(StoreError::Database)
        })?;
        let mut rooms = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(room) = self.snapshot(&id).await? {
                rooms.push(room);
            }
        }
        Ok(rooms)
    }

    pub async fn snapshot(&self, room_id: &str) -> Result<Option<RoomSnapshot>, StoreError> {
        let room_id = validate_room_id(room_id)?;
        self.with_connection(|connection| snapshot_from_connection(connection, &room_id))
    }

    /// Resolve the durable host participant for a Core-authorized host route.
    /// Host routes are already gated by Core's `rooms.*` permission, so they do
    /// not need to carry the guest session cookie through the sidecar.
    pub async fn host_session(&self, room_id: &str) -> Result<RoomSession, StoreError> {
        let room_id = validate_room_id(room_id)?;
        self.with_connection(|connection| {
            let participant = connection
                .query_row(
                    "SELECT id, display_name FROM room_participants WHERE room_id = ?1 AND role = 'host' LIMIT 1",
                    params![room_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            let Some((participant_id, display_name)) = participant else {
                return Err(StoreError::NotFound);
            };
            Ok(RoomSession {
                room_id,
                participant_id,
                role: ParticipantRole::Host,
                display_name,
                secret: String::new(),
            })
        })
    }

    pub async fn exchange_invite(
        &self,
        raw: &str,
        display_name: &str,
    ) -> Result<RoomSession, StoreError> {
        let display_name =
            limits::validate_display_name(display_name).map_err(StoreError::Invalid)?;
        let raw = raw.trim();
        if raw.is_empty() || raw.len() > 128 || raw.chars().any(char::is_control) {
            return Err(StoreError::InviteUnavailable);
        }
        let invite_digest = digest(raw);
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let session_secret = opaque_secret();
        let session_digest = digest(&session_secret);
        let participant_id = opaque_id("member_");
        let expires_at = (now + Duration::seconds(limits::SESSION_TTL_SECS)).to_rfc3339();

        let connection = self.connection.lock().map_err(|_| StoreError::Lock)?;
        let transaction = connection.unchecked_transaction()?;
        let mut invite_statement = transaction
            .prepare("SELECT room_id, digest, expires_at, revoked_at FROM room_invites")?;
        let mut invite_rows = invite_statement.query([])?;
        let mut invite = None;
        while let Some(row) = invite_rows.next()? {
            let stored_digest: Vec<u8> = row.get(1)?;
            if stored_digest
                .as_slice()
                .ct_eq(invite_digest.as_slice())
                .into()
            {
                invite = Some((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ));
                break;
            }
        }
        drop(invite_rows);
        drop(invite_statement);
        let Some((room_id, invite_expires, revoked_at)) = invite else {
            return Err(StoreError::InviteUnavailable);
        };
        if revoked_at.is_some() || !future_timestamp(&invite_expires, now) {
            return Err(StoreError::InviteUnavailable);
        }
        let room_status = transaction.query_row(
            "SELECT status FROM rooms WHERE id = ?1",
            params![room_id],
            |row| row.get::<_, String>(0),
        )?;
        if room_status == "closed" {
            return Err(StoreError::InviteUnavailable);
        }
        let participant_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM room_participants WHERE room_id = ?1",
            params![room_id],
            |row| row.get(0),
        )?;
        if participant_count >= limits::MAX_PARTICIPANTS as i64 {
            return Err(StoreError::Conflict);
        }
        transaction.execute(
            "INSERT INTO room_participants (id, room_id, display_name, role, online, created_at, last_seen_at)
             VALUES (?1, ?2, ?3, 'guest', 1, ?4, ?4)",
            params![participant_id, room_id, display_name, now_text],
        )?;
        transaction.execute(
            "INSERT INTO room_sessions (digest, room_id, participant_id, role, expires_at)
             VALUES (?1, ?2, ?3, 'guest', ?4)",
            params![session_digest, room_id, participant_id, expires_at],
        )?;
        transaction.commit()?;
        Ok(RoomSession {
            room_id,
            participant_id,
            role: ParticipantRole::Guest,
            display_name,
            secret: session_secret,
        })
    }

    /// Rotate the room invite without ever reading the previous raw secret back
    /// from storage. Existing guest sessions are revoked with the old invite.
    pub async fn issue_invite(&self, room_id: &str) -> Result<IssuedInvite, StoreError> {
        let room_id = validate_room_id(room_id)?;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let invite = opaque_secret();
        let expires_at = (now + Duration::seconds(limits::INVITE_TTL_SECS)).to_rfc3339();
        let share_origin = self.with_transaction(|transaction| {
            let room = transaction
                .query_row(
                    "SELECT status, share_origin FROM rooms WHERE id = ?1",
                    params![room_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            let Some((status, share_origin)) = room else {
                return Err(StoreError::NotFound);
            };
            if status == "closed" {
                return Err(StoreError::Closed);
            }
            let changed = transaction.execute(
                "UPDATE room_invites SET digest = ?2, expires_at = ?3, revoked_at = NULL WHERE room_id = ?1",
                params![room_id, digest(&invite), expires_at],
            )?;
            if changed == 0 {
                return Err(StoreError::NotFound);
            }
            transaction.execute(
                "UPDATE room_sessions SET revoked_at = ?2 WHERE room_id = ?1 AND revoked_at IS NULL",
                params![room_id, now_text],
            )?;
            transaction.execute(
                "UPDATE room_participants SET online = 0 WHERE room_id = ?1 AND role = 'guest'",
                params![room_id],
            )?;
            Ok(share_origin)
        })?;
        Ok(IssuedInvite {
            invite,
            share_origin,
        })
    }

    pub async fn resolve_session(&self, raw: &str) -> Result<Option<RoomSession>, StoreError> {
        let raw = raw.trim();
        if raw.is_empty() || raw.len() > 128 {
            return Ok(None);
        }
        let session_digest = digest(raw);
        let now = Utc::now();
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT s.digest, s.room_id, s.participant_id, s.role, p.display_name, s.expires_at, s.revoked_at
                 FROM room_sessions s JOIN room_participants p ON p.id = s.participant_id",
            )?;
            let mut rows = statement.query([])?;
            let mut found = None;
            while let Some(row) = rows.next()? {
                let stored_digest: Vec<u8> = row.get(0)?;
                if stored_digest.as_slice().ct_eq(session_digest.as_slice()).into() {
                    found = Some((
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ));
                    break;
                }
            }
            drop(rows);
            drop(statement);
            let Some((room_id, participant_id, role, display_name, expires_at, revoked_at)) = found else {
                return Ok(None);
            };
            if revoked_at.is_some() || !future_timestamp(&expires_at, now) {
                return Ok(None);
            }
            let role = parse_participant_role(&role).ok_or_else(|| StoreError::Database(rusqlite::Error::InvalidQuery))?;
            Ok(Some(RoomSession {
                room_id,
                participant_id,
                role,
                display_name,
                secret: raw.to_owned(),
            }))
        })
    }

    pub async fn append_user_turn(
        &self,
        session: &RoomSession,
        input: SubmitTurnInput,
    ) -> Result<AcceptedRun, StoreError> {
        if !matches!(session.role, ParticipantRole::Host | ParticipantRole::Guest) {
            return Err(StoreError::Forbidden);
        }
        let text = limits::validate_prompt(&input.text).map_err(StoreError::Invalid)?;
        let idempotency_key = input
            .idempotency_key
            .map(|value| {
                limits::validate_non_empty(
                    &value,
                    limits::MAX_IDEMPOTENCY_KEY_CHARS,
                    "idempotencyKey",
                )
            })
            .transpose()
            .map_err(StoreError::Invalid)?;
        let room_id = session.room_id.clone();
        let (run_id, message_id) = {
            let connection = self.connection.lock().map_err(|_| StoreError::Lock)?;
            let transaction = connection.unchecked_transaction()?;
            let status = transaction
                .query_row(
                    "SELECT status FROM rooms WHERE id = ?1",
                    params![room_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let Some(status) = status else {
                return Err(StoreError::NotFound);
            };
            if status == "closed" {
                return Err(StoreError::Closed);
            }
            if let Some(key) = idempotency_key.as_deref() {
                let existing = transaction
                    .query_row(
                        "SELECT id, input_text FROM room_runs WHERE room_id = ?1 AND idempotency_key = ?2",
                        params![room_id, key],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?;
                if let Some((run_id, previous_text)) = existing {
                    if previous_text != text {
                        return Err(StoreError::Conflict);
                    }
                    transaction.commit()?;
                    (run_id, None)
                } else {
                    let running: i64 = transaction.query_row(
                        "SELECT COUNT(*) FROM room_runs WHERE room_id = ?1 AND status IN ('queued', 'running')",
                        params![room_id],
                        |row| row.get(0),
                    )?;
                    if running > 0 {
                        return Err(StoreError::RoomBusy);
                    }
                    let now = Utc::now().to_rfc3339();
                    let message_id = opaque_id("msg_");
                    let run_id = opaque_id("run_");
                    let request_id = opaque_id("req_");
                    transaction.execute(
                        "INSERT INTO room_messages (id, room_id, role, text, participant_id, created_at)
                         VALUES (?1, ?2, 'user', ?3, ?4, ?5)",
                        params![message_id, room_id, text, session.participant_id, now],
                    )?;
                    transaction.execute(
                        "INSERT INTO room_runs (id, room_id, request_id, idempotency_key, input_text, status, started_at, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?6)",
                        params![run_id, room_id, request_id, idempotency_key, text, now],
                    )?;
                    transaction.execute(
                        "UPDATE rooms SET status = 'running', updated_at = ?2 WHERE id = ?1",
                        params![room_id, now],
                    )?;
                    transaction.commit()?;
                    (run_id, Some(message_id))
                }
            } else {
                let running: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM room_runs WHERE room_id = ?1 AND status IN ('queued', 'running')",
                    params![room_id],
                    |row| row.get(0),
                )?;
                if running > 0 {
                    return Err(StoreError::RoomBusy);
                }
                let now = Utc::now().to_rfc3339();
                let message_id = opaque_id("msg_");
                let run_id = opaque_id("run_");
                let request_id = opaque_id("req_");
                transaction.execute(
                    "INSERT INTO room_messages (id, room_id, role, text, participant_id, created_at)
                     VALUES (?1, ?2, 'user', ?3, ?4, ?5)",
                    params![message_id, room_id, text, session.participant_id, now],
                )?;
                transaction.execute(
                    "INSERT INTO room_runs (id, room_id, request_id, idempotency_key, input_text, status, started_at, created_at)
                     VALUES (?1, ?2, ?3, NULL, ?4, 'running', ?5, ?5)",
                    params![run_id, room_id, request_id, text, now],
                )?;
                transaction.execute(
                    "UPDATE rooms SET status = 'running', updated_at = ?2 WHERE id = ?1",
                    params![room_id, now],
                )?;
                transaction.commit()?;
                (run_id, Some(message_id))
            }
        };
        let snapshot = self.snapshot(&room_id).await?.ok_or(StoreError::NotFound)?;
        let run = match message_id {
            Some(_) => snapshot.current_run.clone().ok_or(StoreError::NotFound)?,
            None => self
                .run_by_id_sync(&room_id, &run_id)?
                .ok_or(StoreError::NotFound)?,
        };
        let user_message = snapshot
            .messages
            .iter()
            .rev()
            .find(|message| {
                message.role == MessageRole::User
                    && message.text == text
                    && message_id
                        .as_deref()
                        .is_none_or(|message_id| message.id == message_id)
            })
            .cloned()
            .ok_or(StoreError::NotFound)?;
        Ok(AcceptedRun {
            run,
            user_message,
            snapshot,
        })
    }

    pub async fn append_delta(
        &self,
        room_id: &str,
        run_id: &str,
        delta: &str,
    ) -> Result<(), StoreError> {
        let delta = limits::validate_delta(delta).map_err(StoreError::Invalid)?;
        self.with_connection(|connection| {
            let current: Option<String> = connection.query_row(
                "SELECT partial_text FROM room_runs WHERE id = ?1 AND room_id = ?2 AND status = 'running'",
                params![run_id, room_id],
                |row| row.get(0),
            ).optional()?;
            let Some(current) = current else { return Err(StoreError::NotFound); };
            let combined = format!("{current}{delta}");
            if combined.chars().count() > limits::MAX_MESSAGE_CHARS { return Err(StoreError::Invalid("assistant output is too long".to_owned())); }
            connection.execute(
                "UPDATE room_runs SET partial_text = ?1 WHERE id = ?2 AND room_id = ?3 AND status = 'running'",
                params![combined, run_id, room_id],
            )?;
            connection.execute(
                "UPDATE rooms SET updated_at = ?2 WHERE id = ?1",
                params![room_id, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
    }

    pub async fn finish_run(
        &self,
        room_id: &str,
        run_id: &str,
        terminal: RunTerminal,
    ) -> Result<RoomSnapshot, StoreError> {
        let room_id = validate_room_id(room_id)?;
        {
            let connection = self.connection.lock().map_err(|_| StoreError::Lock)?;
            let transaction = connection.unchecked_transaction()?;
            let current = transaction
                .query_row(
                    "SELECT status, partial_text FROM room_runs WHERE id = ?1 AND room_id = ?2",
                    params![run_id, room_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            let Some((current_status, partial_text)) = current else {
                return Err(StoreError::NotFound);
            };
            if matches!(current_status.as_str(), "queued" | "running") {
                let now = Utc::now().to_rfc3339();
                if terminal.status == RunStatus::Completed && !partial_text.is_empty() {
                    transaction.execute(
                        "INSERT INTO room_messages (id, room_id, role, text, participant_id, created_at)
                         VALUES (?1, ?2, 'assistant', ?3, NULL, ?4)",
                        params![opaque_id("msg_"), room_id, partial_text, now],
                    )?;
                }
                let status = run_status_text(terminal.status);
                transaction.execute(
                    "UPDATE room_runs SET status = ?1, error_code = ?2, error_message = ?3, finished_at = ?4 WHERE id = ?5 AND room_id = ?6",
                    params![status, terminal.error_code, terminal.error_message, now, run_id, room_id],
                )?;
                let room_status = if status == "failed" { "failed" } else { "idle" };
                transaction.execute(
                    "UPDATE rooms SET status = CASE WHEN status = 'closed' THEN 'closed' ELSE ?2 END, updated_at = ?3 WHERE id = ?1",
                    params![room_id, room_status, now],
                )?;
                trim_runs(&transaction, &room_id)?;
                trim_messages(&transaction, &room_id)?;
            }
            transaction.commit()?;
        }
        self.snapshot(&room_id).await?.ok_or(StoreError::NotFound)
    }

    pub async fn stop_run(&self, room_id: &str, run_id: &str) -> Result<(), StoreError> {
        let room_id = validate_room_id(room_id)?;
        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE room_runs SET status = 'canceled', error_code = 'canceled', error_message = 'generation stopped', finished_at = ?1
                 WHERE id = ?2 AND room_id = ?3 AND status IN ('queued', 'running')",
                params![Utc::now().to_rfc3339(), run_id, room_id],
            )?;
            if changed == 0 { return Err(StoreError::NotFound); }
            connection.execute(
                "UPDATE rooms SET status = CASE WHEN status = 'closed' THEN 'closed' ELSE 'idle' END, updated_at = ?2 WHERE id = ?1",
                params![room_id, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
    }

    pub async fn close_room(&self, room_id: &str) -> Result<(), StoreError> {
        let room_id = validate_room_id(room_id)?;
        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE rooms SET status = 'closed', updated_at = ?2 WHERE id = ?1 AND status != 'closed'",
                params![room_id, Utc::now().to_rfc3339()],
            )?;
            if changed == 0 { return Err(StoreError::NotFound); }
            connection.execute(
                "UPDATE room_invites SET revoked_at = ?2 WHERE room_id = ?1 AND revoked_at IS NULL",
                params![room_id, Utc::now().to_rfc3339()],
            )?;
            connection.execute(
                "UPDATE room_sessions SET revoked_at = ?2 WHERE room_id = ?1 AND revoked_at IS NULL",
                params![room_id, Utc::now().to_rfc3339()],
            )?;
            connection.execute(
                "UPDATE room_participants SET online = 0 WHERE room_id = ?1",
                params![room_id],
            )?;
            Ok(())
        })
    }

    pub async fn revoke_invite(&self, room_id: &str) -> Result<(), StoreError> {
        let room_id = validate_room_id(room_id)?;
        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE room_invites SET revoked_at = ?2 WHERE room_id = ?1 AND revoked_at IS NULL",
                params![room_id, Utc::now().to_rfc3339()],
            )?;
            if changed == 0 { return Err(StoreError::NotFound); }
            connection.execute(
                "UPDATE room_sessions SET revoked_at = ?2 WHERE room_id = ?1 AND revoked_at IS NULL",
                params![room_id, Utc::now().to_rfc3339()],
            )?;
            connection.execute(
                "UPDATE room_participants SET online = 0 WHERE room_id = ?1 AND role = 'guest'",
                params![room_id],
            )?;
            Ok(())
        })
    }

    pub async fn recover_running(&self) -> Result<usize, StoreError> {
        self.with_connection(|connection| {
            let now = Utc::now().to_rfc3339();
            let changed = connection.execute(
                "UPDATE room_runs SET status = 'failed', error_code = 'nodeRestarted', error_message = 'generation interrupted by node restart', finished_at = ?1 WHERE status IN ('queued', 'running')",
                params![now],
            )?;
            connection.execute(
                "UPDATE rooms SET status = 'failed', updated_at = ?1 WHERE status = 'running'",
                params![now],
            )?;
            Ok(changed)
        })
    }

    pub async fn raw_invite_for_test(&self, room_id: &str) -> Result<Option<String>, StoreError> {
        self.with_connection(|connection| {
            // The schema deliberately has only a digest. This helper proves that no raw invite
            // can be read back by a future handler or a test accidentally exposing it.
            let has_row: Option<i64> = connection
                .query_row(
                    "SELECT 1 FROM room_invites WHERE room_id = ?1",
                    params![room_id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(has_row.and(None))
        })
    }

    fn run_by_id_sync(&self, room_id: &str, run_id: &str) -> Result<Option<RoomRun>, StoreError> {
        self.with_connection(|connection| {
            connection.query_row(
                "SELECT id, request_id, idempotency_key, status, partial_text, error_code, error_message, started_at, finished_at
                 FROM room_runs WHERE room_id = ?1 AND id = ?2",
                params![room_id, run_id],
                |row| row_to_run(row),
            ).optional().map_err(StoreError::Database)
        })
    }

    fn with_transaction<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut connection = self.connection.lock().map_err(|_| StoreError::Lock)?;
        let transaction = connection.transaction()?;
        let result = f(&transaction)?;
        transaction.commit()?;
        Ok(result)
    }
}

fn snapshot_from_connection(
    connection: &Connection,
    room_id: &str,
) -> Result<Option<RoomSnapshot>, StoreError> {
    let room = connection
        .query_row(
            "SELECT status, model_id, updated_at FROM rooms WHERE id = ?1",
            params![room_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((status, model_id, updated_at)) = room else {
        return Ok(None);
    };
    let messages = load_messages(connection, room_id)?;
    let current_run = connection.query_row(
        "SELECT id, request_id, idempotency_key, status, partial_text, error_code, error_message, started_at, finished_at
         FROM room_runs WHERE room_id = ?1 AND status IN ('queued', 'running', 'failed') ORDER BY created_at DESC, id DESC LIMIT 1",
        params![room_id],
        |row| row_to_run(row),
    ).optional()?;
    let participants = load_participants(connection, room_id)?;
    Ok(Some(RoomSnapshot {
        contract: CONTRACT.to_owned(),
        schema_version: 1,
        id: room_id.to_owned(),
        status: parse_room_status(&status)
            .ok_or_else(|| StoreError::Database(rusqlite::Error::InvalidQuery))?,
        model_id,
        engine: ENGINE.to_owned(),
        messages,
        current_run,
        participants,
        updated_at,
    }))
}

fn load_messages(connection: &Connection, room_id: &str) -> Result<Vec<RoomMessage>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT id, role, text, participant_id, created_at FROM room_messages WHERE room_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
    )?;
    let mut messages = statement
        .query_map(params![room_id, MAX_HISTORY_MESSAGES as i64], |row| {
            Ok(RoomMessage {
                id: row.get(0)?,
                role: parse_message_role(&row.get::<_, String>(1)?)
                    .ok_or(rusqlite::Error::InvalidQuery)?,
                text: row.get(2)?,
                participant_id: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    messages.reverse();
    Ok(messages)
}

fn load_participants(
    connection: &Connection,
    room_id: &str,
) -> Result<Vec<RoomParticipantSummary>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT id, display_name, role, online FROM room_participants WHERE room_id = ?1 ORDER BY created_at ASC, id ASC",
    )?;
    let rows = statement.query_map(params![room_id], |row| {
        Ok(RoomParticipantSummary {
            id: row.get(0)?,
            display_name: row.get(1)?,
            role: parse_participant_role(&row.get::<_, String>(2)?)
                .ok_or(rusqlite::Error::InvalidQuery)?,
            online: row.get::<_, i64>(3)? != 0,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::Database)
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoomRun> {
    Ok(RoomRun {
        run_id: row.get(0)?,
        request_id: row.get(1)?,
        idempotency_key: row.get(2)?,
        status: parse_run_status(&row.get::<_, String>(3)?).ok_or(rusqlite::Error::InvalidQuery)?,
        partial_text: row.get(4)?,
        error_code: row.get(5)?,
        error_message: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
    })
}

fn trim_runs(transaction: &Transaction<'_>, room_id: &str) -> Result<(), StoreError> {
    transaction.execute(
        "DELETE FROM room_runs WHERE room_id = ?1 AND id NOT IN (SELECT id FROM room_runs WHERE room_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2)",
        params![room_id, MAX_RETAINED_RUNS as i64],
    )?;
    Ok(())
}

fn trim_messages(transaction: &Transaction<'_>, room_id: &str) -> Result<(), StoreError> {
    transaction.execute(
        "DELETE FROM room_messages WHERE room_id = ?1 AND id NOT IN (SELECT id FROM room_messages WHERE room_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2)",
        params![room_id, MAX_HISTORY_MESSAGES as i64],
    )?;
    Ok(())
}

fn opaque_id(prefix: &str) -> String {
    format!("{}{}", prefix, Uuid::new_v4().simple())
}

fn opaque_secret() -> String {
    Uuid::new_v4().simple().to_string()
}

fn digest(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

fn future_timestamp(value: &str, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc) > now)
        .unwrap_or(false)
}

fn validate_room_id(value: &str) -> Result<String, StoreError> {
    let value = value.trim();
    if value.len() < 13
        || value.len() > 105
        || !value.starts_with("room_")
        || value
            .chars()
            .any(|character| !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-')))
    {
        return Err(StoreError::Invalid("roomId is invalid".to_owned()));
    }
    Ok(value.to_owned())
}

fn parse_room_status(value: &str) -> Option<RoomStatus> {
    match value {
        "idle" => Some(RoomStatus::Idle),
        "running" => Some(RoomStatus::Running),
        "failed" => Some(RoomStatus::Failed),
        "closed" => Some(RoomStatus::Closed),
        _ => None,
    }
}

fn parse_run_status(value: &str) -> Option<RunStatus> {
    match value {
        "queued" => Some(RunStatus::Queued),
        "running" => Some(RunStatus::Running),
        "completed" => Some(RunStatus::Completed),
        "failed" => Some(RunStatus::Failed),
        "canceled" => Some(RunStatus::Canceled),
        _ => None,
    }
}

fn run_status_text(value: RunStatus) -> &'static str {
    match value {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Canceled => "canceled",
    }
}

fn parse_message_role(value: &str) -> Option<MessageRole> {
    match value {
        "user" => Some(MessageRole::User),
        "assistant" => Some(MessageRole::Assistant),
        _ => None,
    }
}

fn parse_participant_role(value: &str) -> Option<ParticipantRole> {
    match value {
        "host" => Some(ParticipantRole::Host),
        "guest" => Some(ParticipantRole::Guest),
        "viewer" => Some(ParticipantRole::Viewer),
        _ => None,
    }
}
