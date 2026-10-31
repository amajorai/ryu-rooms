//! The Rooms store (`rooms.db`).
//!
//! At rest (see [`crate::seal_keys`]): `room_messages.text`,
//! `room_runs.input_text`, `room_runs.partial_text`, `room_runs.error_message`
//! and `room_participants.display_name` are sealed per value (`encb:v1:`, store
//! `rooms.db`, bound to the row id) under the `@ryu/rooms` plugin subkey. The
//! streamed partial answer is opened, extended and re-sealed on every
//! `append_delta`. The idempotency check compares opened plaintext in Rust.
//! Ids, statuses, timestamps and token digests stay plaintext. Legacy plaintext
//! rows are sealed when the store opens, then freed pages are scrubbed. Every
//! connection runs with `secure_delete = ON`.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use ryu_crypto::content_sqlite::{
    enable_secure_delete, finish_store, migrate_columns, run_pending_vacuum, ColumnPlan, KeyKind,
};
use ryu_crypto::ContentContext;

use crate::seal_keys::{sqlite_store_seal, PluginKeys, StoreSeal};

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

/// The plugin id (the sealing key's scope).
pub const PLUGIN_ID: &str = "@ryu/rooms";
/// The registered content-store name (part of every key derivation; never rename).
pub const STORE: &str = "rooms.db";

fn ctx(table: &str, column: &str, id: &str) -> ContentContext {
    ContentContext::for_id(format!("{table}.{column}"), id)
}

/// The sealed columns, for the open-time migration.
pub fn column_plans() -> anyhow::Result<Vec<ColumnPlan>> {
    Ok(vec![
        ColumnPlan::text("room_messages", "text", "id", KeyKind::Text)?,
        ColumnPlan::text("room_runs", "input_text", "id", KeyKind::Text)?,
        ColumnPlan::text("room_runs", "partial_text", "id", KeyKind::Text)?,
        ColumnPlan::text("room_runs", "error_message", "id", KeyKind::Text)?,
        ColumnPlan::text("room_participants", "display_name", "id", KeyKind::Text)?,
    ])
}
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
    #[error("content sealing failed: {0}")]
    Seal(String),
}

fn seal_error(error: anyhow::Error) -> StoreError {
    StoreError::Seal(format!("{error:#}"))
}

impl StoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Database(_) | Self::Lock | Self::Seal(_) => "storeUnavailable",
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

/// How a store seals, plus its unreadable-row counter.
#[derive(Clone)]
struct Sealing {
    seal: StoreSeal,
    unreadable: Arc<AtomicU64>,
}

impl Sealing {
    fn seal(&self, value: &str, context: &ContentContext) -> Result<String, StoreError> {
        self.seal.seal_text(value, context).map_err(seal_error)
    }

    fn seal_opt(
        &self,
        value: Option<&str>,
        context: &ContentContext,
    ) -> Result<Option<String>, StoreError> {
        self.seal.seal_opt(value, context).map_err(seal_error)
    }

    fn open(&self, stored: &str, context: &ContentContext) -> Result<String, StoreError> {
        self.seal.open_text(stored, context).map_err(seal_error)
    }

    fn open_opt(
        &self,
        stored: Option<&str>,
        context: &ContentContext,
    ) -> Result<Option<String>, StoreError> {
        self.seal.open_opt(stored, context).map_err(seal_error)
    }

    fn skip(&self, what: &str, id: &str, error: &StoreError) {
        let total = self.unreadable.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::warn!(id = %id, total, "skipping a {what} that does not open: {error}");
    }
}

#[derive(Clone)]
pub struct RoomStore {
    connection: Arc<Mutex<Connection>>,
    sealing: Sealing,
}

impl RoomStore {
    /// Open the store, resolving this process's sealing key with the store's
    /// directory as the plugin data dir (see [`crate::seal_keys`]).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = path
            .as_ref()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let keys = PluginKeys::resolve(PLUGIN_ID, &dir).map_err(seal_error)?;
        Self::open_with_keys(path, &keys)
    }

    /// [`Self::open`] with explicit keys.
    pub fn open_with_keys(path: impl AsRef<Path>, keys: &PluginKeys) -> Result<Self, StoreError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|_| StoreError::Lock)?;
        }
        let connection = Connection::open(path.as_ref())?;
        Self::from_connection(connection, keys, Some(path.as_ref()))
    }

    /// An in-memory store (tests). Nothing reaches the disk, so it does not seal.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::open_in_memory_with_keys(&PluginKeys::compat())
    }

    /// An in-memory store that seals under `keys` (tests).
    pub fn open_in_memory_with_keys(keys: &PluginKeys) -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?, keys, None)
    }

    fn from_connection(
        mut connection: Connection,
        keys: &PluginKeys,
        path: Option<&Path>,
    ) -> Result<Self, StoreError> {
        enable_secure_delete(&connection).map_err(seal_error)?;
        initialize(&connection)?;
        let seal = sqlite_store_seal(keys, &connection, STORE).map_err(seal_error)?;
        if let StoreSeal::Sealing(cipher) = &seal {
            if let Some(path) = path {
                if let Err(error) = run_pending_vacuum(&connection, path, STORE) {
                    tracing::warn!("rooms.db: deferred compaction failed: {error:#}");
                }
            }
            let report = migrate_columns(
                &mut connection,
                cipher,
                &column_plans().map_err(seal_error)?,
            )
            .map_err(seal_error)?;
            if report.sealed > 0 {
                tracing::info!(count = report.sealed, "rooms.db: sealed legacy values");
                if let Some(path) = path {
                    if let Err(error) = finish_store(&connection, path, STORE) {
                        tracing::warn!("rooms.db: scrubbing freed pages failed: {error:#}");
                    }
                }
            }
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            sealing: Sealing {
                seal,
                unreadable: Arc::new(AtomicU64::new(0)),
            },
        })
    }

    /// The sealing health item: `{state, detail?, unreadable_rows}`.
    pub fn seal_health(&self) -> serde_json::Value {
        let mut health = self.sealing.seal.health();
        health["unreadable_rows"] = self.sealing.unreadable.load(Ordering::Relaxed).into();
        health
    }

    fn with_connection<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let connection = self.connection.lock().map_err(|_| StoreError::Lock)?;
        f(&connection)
    }
}

fn initialize(connection: &Connection) -> Result<(), StoreError> {
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
}

impl RoomStore {
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
        let host_name = self.sealing.seal(
            "Host",
            &ctx("room_participants", "display_name", &participant_id),
        )?;

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
                 VALUES (?1, ?2, ?3, 'host', 1, ?4, ?4)",
                params![participant_id, room_id, host_name, now_text],
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
        self.with_connection(|connection| {
            snapshot_from_connection(connection, &self.sealing, &room_id)
        })
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
            let display_name = self.sealing.open(
                &display_name,
                &ctx("room_participants", "display_name", &participant_id),
            )?;
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
        let sealed_name = self.sealing.seal(
            &display_name,
            &ctx("room_participants", "display_name", &participant_id),
        )?;

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
        // The cap is on live participants: the host plus guests that still
        // hold an unrevoked, unexpired session.
        let mut live_statement = transaction.prepare(
            "SELECT p.id, p.role, s.expires_at, s.revoked_at
             FROM room_participants p LEFT JOIN room_sessions s ON s.participant_id = p.id
             WHERE p.room_id = ?1",
        )?;
        let mut live_rows = live_statement.query(params![room_id])?;
        let mut live_participants = std::collections::HashSet::new();
        while let Some(row) = live_rows.next()? {
            let id: String = row.get(0)?;
            let role: String = row.get(1)?;
            let expires_at: Option<String> = row.get(2)?;
            let revoked_at: Option<String> = row.get(3)?;
            let live_session = revoked_at.is_none()
                && expires_at.is_some_and(|value| future_timestamp(&value, now));
            if role == "host" || live_session {
                live_participants.insert(id);
            }
        }
        drop(live_rows);
        drop(live_statement);
        if live_participants.len() >= limits::MAX_PARTICIPANTS {
            return Err(StoreError::Conflict);
        }
        transaction.execute(
            "INSERT INTO room_participants (id, room_id, display_name, role, online, created_at, last_seen_at)
             VALUES (?1, ?2, ?3, 'guest', 1, ?4, ?4)",
            params![participant_id, room_id, sealed_name, now_text],
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
            // Revoked guests must exchange the new invite as new participants;
            // drop their rows (sessions cascade) so they stop counting and listing.
            transaction.execute(
                "DELETE FROM room_participants WHERE room_id = ?1 AND role = 'guest'",
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
            let display_name = self.sealing.open(
                &display_name,
                &ctx("room_participants", "display_name", &participant_id),
            )?;
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
                    // Sealed values are randomized: compare the opened text.
                    let previous_text = self
                        .sealing
                        .open(&previous_text, &ctx("room_runs", "input_text", &run_id))?;
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
                    let sealed = self.seal_turn(&message_id, &run_id, &text)?;
                    transaction.execute(
                        "INSERT INTO room_messages (id, room_id, role, text, participant_id, created_at)
                         VALUES (?1, ?2, 'user', ?3, ?4, ?5)",
                        params![message_id, room_id, sealed.message, session.participant_id, now],
                    )?;
                    transaction.execute(
                        "INSERT INTO room_runs (id, room_id, request_id, idempotency_key, input_text, status, partial_text, started_at, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?7, ?7)",
                        params![run_id, room_id, request_id, idempotency_key, sealed.input, sealed.partial, now],
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
                let sealed = self.seal_turn(&message_id, &run_id, &text)?;
                transaction.execute(
                    "INSERT INTO room_messages (id, room_id, role, text, participant_id, created_at)
                     VALUES (?1, ?2, 'user', ?3, ?4, ?5)",
                    params![message_id, room_id, sealed.message, session.participant_id, now],
                )?;
                transaction.execute(
                    "INSERT INTO room_runs (id, room_id, request_id, idempotency_key, input_text, status, partial_text, started_at, created_at)
                     VALUES (?1, ?2, ?3, NULL, ?4, 'running', ?5, ?6, ?6)",
                    params![run_id, room_id, request_id, sealed.input, sealed.partial, now],
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
            let context = ctx("room_runs", "partial_text", run_id);
            let current = self.sealing.open(&current, &context)?;
            let combined = format!("{current}{delta}");
            if combined.chars().count() > limits::MAX_MESSAGE_CHARS { return Err(StoreError::Invalid("assistant output is too long".to_owned())); }
            // Re-sealed on every delta (a fresh nonce each time): bounded by
            // MAX_MESSAGE_CHARS, so one open and one seal of a short buffer.
            let combined = self.sealing.seal(&combined, &context)?;
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
                let partial_text = self
                    .sealing
                    .open(&partial_text, &ctx("room_runs", "partial_text", run_id))?;
                if terminal.status == RunStatus::Completed && !partial_text.is_empty() {
                    // A copy to a new id: opened above, sealed under the message id.
                    let message_id = opaque_id("msg_");
                    let text = self
                        .sealing
                        .seal(&partial_text, &ctx("room_messages", "text", &message_id))?;
                    transaction.execute(
                        "INSERT INTO room_messages (id, room_id, role, text, participant_id, created_at)
                         VALUES (?1, ?2, 'assistant', ?3, NULL, ?4)",
                        params![message_id, room_id, text, now],
                    )?;
                }
                let error_message = self.sealing.seal_opt(
                    terminal.error_message.as_deref(),
                    &ctx("room_runs", "error_message", run_id),
                )?;
                let status = run_status_text(terminal.status);
                transaction.execute(
                    "UPDATE room_runs SET status = ?1, error_code = ?2, error_message = ?3, finished_at = ?4 WHERE id = ?5 AND room_id = ?6",
                    params![status, terminal.error_code, error_message, now, run_id, room_id],
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
        let message = self.sealing.seal(
            "generation stopped",
            &ctx("room_runs", "error_message", run_id),
        )?;
        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE room_runs SET status = 'canceled', error_code = 'canceled', error_message = ?4, finished_at = ?1
                 WHERE id = ?2 AND room_id = ?3 AND status IN ('queued', 'running')",
                params![Utc::now().to_rfc3339(), run_id, room_id, message],
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
            // Revoked guests must exchange the invite again as new participants;
            // drop their rows (sessions cascade) so they stop counting and listing.
            connection.execute(
                "DELETE FROM room_participants WHERE room_id = ?1 AND role = 'guest'",
                params![room_id],
            )?;
            Ok(())
        })
    }

    pub async fn recover_running(&self) -> Result<usize, StoreError> {
        self.with_connection(|connection| {
            let now = Utc::now().to_rfc3339();
            let ids = {
                let mut statement = connection
                    .prepare("SELECT id FROM room_runs WHERE status IN ('queued', 'running')")?;
                let ids = statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                ids
            };
            let mut changed = 0;
            // An unhealthy store (sealed under a key this process does not
            // hold) refuses every seal: recovery then clears the message and
            // keeps `error_code = 'nodeRestarted'`, so startup never fails on
            // it (a restart loop otherwise).
            let writable = self.sealing.seal.ensure_writable().is_ok();
            for id in ids {
                // Each run's message is sealed under its own id.
                let message = if writable {
                    Some(self.sealing.seal(
                        "generation interrupted by node restart",
                        &ctx("room_runs", "error_message", &id),
                    )?)
                } else {
                    None
                };
                changed += connection.execute(
                    "UPDATE room_runs SET status = 'failed', error_code = 'nodeRestarted', error_message = ?2, finished_at = ?1 WHERE id = ?3 AND status IN ('queued', 'running')",
                    params![now, message, id],
                )?;
            }
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
            ).optional()?
            .map(|run| open_run(&self.sealing, run))
            .transpose()
        })
    }

    /// Seal a user turn: the message text, the run input and the empty partial.
    fn seal_turn(
        &self,
        message_id: &str,
        run_id: &str,
        text: &str,
    ) -> Result<SealedTurn, StoreError> {
        Ok(SealedTurn {
            message: self
                .sealing
                .seal(text, &ctx("room_messages", "text", message_id))?,
            input: self
                .sealing
                .seal(text, &ctx("room_runs", "input_text", run_id))?,
            partial: self
                .sealing
                .seal("", &ctx("room_runs", "partial_text", run_id))?,
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

/// The sealed cells of one user turn.
struct SealedTurn {
    message: String,
    input: String,
    partial: String,
}

fn open_run(sealing: &Sealing, mut run: RoomRun) -> Result<RoomRun, StoreError> {
    run.partial_text = sealing.open(
        &run.partial_text,
        &ctx("room_runs", "partial_text", &run.run_id),
    )?;
    run.error_message = sealing.open_opt(
        run.error_message.as_deref(),
        &ctx("room_runs", "error_message", &run.run_id),
    )?;
    Ok(run)
}

/// [`open_run`] for the room snapshot: a run whose text does not open is
/// counted and shown without its partial text or message, so one unreadable
/// run never makes the whole room unavailable.
fn open_run_or_blank(sealing: &Sealing, run: RoomRun) -> RoomRun {
    let mut blank = run.clone();
    match open_run(sealing, run) {
        Ok(run) => run,
        Err(error) => {
            sealing.skip("run", &blank.run_id, &error);
            blank.partial_text = String::new();
            blank.error_message = None;
            blank
        }
    }
}

fn snapshot_from_connection(
    connection: &Connection,
    sealing: &Sealing,
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
    let messages = load_messages(connection, sealing, room_id)?;
    let current_run = connection.query_row(
        "SELECT id, request_id, idempotency_key, status, partial_text, error_code, error_message, started_at, finished_at
         FROM room_runs WHERE room_id = ?1 AND status IN ('queued', 'running', 'failed') ORDER BY created_at DESC, id DESC LIMIT 1",
        params![room_id],
        |row| row_to_run(row),
    ).optional()?
    .map(|run| open_run_or_blank(sealing, run));
    let participants = load_participants(connection, sealing, room_id)?;
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

fn load_messages(
    connection: &Connection,
    sealing: &Sealing,
    room_id: &str,
) -> Result<Vec<RoomMessage>, StoreError> {
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
    // A message that does not open is skipped and counted, never fatal.
    let mut opened = Vec::with_capacity(messages.len());
    for mut message in messages {
        match sealing.open(&message.text, &ctx("room_messages", "text", &message.id)) {
            Ok(text) => {
                message.text = text;
                opened.push(message);
            }
            Err(error) => sealing.skip("message", &message.id, &error),
        }
    }
    Ok(opened)
}

fn load_participants(
    connection: &Connection,
    sealing: &Sealing,
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
    let rows = rows.collect::<Result<Vec<_>, _>>()?;
    let mut opened = Vec::with_capacity(rows.len());
    for mut participant in rows {
        match sealing.open(
            &participant.display_name,
            &ctx("room_participants", "display_name", &participant.id),
        ) {
            Ok(name) => {
                participant.display_name = name;
                opened.push(participant);
            }
            Err(error) => sealing.skip("participant", &participant.id, &error),
        }
    }
    Ok(opened)
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

#[cfg(test)]
mod seal_tests {
    use super::*;
    use crate::seal_keys::KeySource;
    use base64::Engine as _;
    use ryu_crypto::{ContentCipher, PluginCipher};

    const SECRET: &str = "zebra-room-marker";
    const ANSWER: &str = "quokka-answer-marker";
    const GUEST: &str = "Wombat Guestname";

    fn keys(n: u8) -> PluginKeys {
        let subkey = base64::engine::general_purpose::STANDARD.encode([n; 32]);
        PluginKeys::sealing(
            PluginCipher::from_subkey_b64(PLUGIN_ID, &subkey).unwrap(),
            KeySource::Delivered,
        )
    }

    fn db_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!("ryu-rooms-seal-{}", Uuid::new_v4().simple()))
            .join(STORE)
    }

    fn disk_bytes(path: &Path) -> Vec<u8> {
        let mut bytes = std::fs::read(path).unwrap_or_default();
        for suffix in ["-wal", "-journal"] {
            let mut side = path.as_os_str().to_owned();
            side.push(suffix);
            bytes.extend(std::fs::read(PathBuf::from(side)).unwrap_or_default());
        }
        bytes
    }

    fn contains(haystack: &[u8], needle: &str) -> bool {
        haystack
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
    }

    /// A room with one guest turn and a streamed, finished answer.
    async fn conversation(store: &RoomStore) -> String {
        let created = store
            .create_room(CreateRoomInput {
                model_id: "model-a".to_owned(),
                share_origin: "http://192.168.1.20:7980".to_owned(),
            })
            .await
            .unwrap();
        let session = store.exchange_invite(&created.invite, GUEST).await.unwrap();
        let accepted = store
            .append_user_turn(
                &session,
                SubmitTurnInput {
                    text: SECRET.to_owned(),
                    idempotency_key: Some("k1".to_owned()),
                },
            )
            .await
            .unwrap();
        // The idempotency check compares opened text.
        store
            .append_user_turn(
                &session,
                SubmitTurnInput {
                    text: SECRET.to_owned(),
                    idempotency_key: Some("k1".to_owned()),
                },
            )
            .await
            .unwrap();
        let run_id = accepted.run.run_id.clone();
        store
            .append_delta(&created.room.id, &run_id, "quokka-")
            .await
            .unwrap();
        store
            .append_delta(&created.room.id, &run_id, "answer-marker")
            .await
            .unwrap();
        store
            .finish_run(
                &created.room.id,
                &run_id,
                RunTerminal {
                    status: RunStatus::Completed,
                    error_code: None,
                    error_message: None,
                },
            )
            .await
            .unwrap();
        created.room.id
    }

    fn raw_texts(store: &RoomStore) -> Vec<String> {
        let connection = store.connection.lock().unwrap();
        let mut statement = connection
            .prepare(
                "SELECT text FROM room_messages UNION ALL SELECT input_text FROM room_runs
                 UNION ALL SELECT partial_text FROM room_runs UNION ALL SELECT display_name FROM room_participants",
            )
            .unwrap();
        let mut rows: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows.sort();
        rows
    }

    #[tokio::test]
    async fn transcripts_are_sealed_on_disk_and_read_back() {
        let path = db_path();
        let store = RoomStore::open_with_keys(&path, &keys(1)).unwrap();
        let room_id = conversation(&store).await;
        let snapshot = store.snapshot(&room_id).await.unwrap().unwrap();
        let texts: Vec<_> = snapshot.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, [SECRET, ANSWER]);
        assert!(snapshot
            .participants
            .iter()
            .any(|p| p.display_name == GUEST));
        assert!(snapshot
            .participants
            .iter()
            .any(|p| p.display_name == "Host"));
        let bytes = disk_bytes(&path);
        for needle in [SECRET, ANSWER, GUEST] {
            assert!(!contains(&bytes, needle), "{needle} on disk");
        }
        let connection = store.connection.lock().unwrap();
        let cipher = keys(1).content_cipher(STORE).unwrap();
        ryu_crypto::content_sqlite::assert_all_open(&connection, &cipher, &column_plans().unwrap())
            .unwrap();
    }

    #[tokio::test]
    async fn legacy_plaintext_is_migrated_once_and_still_reads() {
        let path = db_path();
        let room_id = {
            let store = RoomStore::open_with_keys(&path, &PluginKeys::compat()).unwrap();
            assert_eq!(store.seal_health()["state"], "compat");
            conversation(&store).await
        };
        assert!(contains(&disk_bytes(&path), SECRET));
        let store = RoomStore::open_with_keys(&path, &keys(2)).unwrap();
        let snapshot = store.snapshot(&room_id).await.unwrap().unwrap();
        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.messages[0].text, SECRET);
        let first = raw_texts(&store);
        assert!(first
            .iter()
            .all(|value| ContentCipher::is_sealed_text(value)));
        drop(store);
        assert!(!contains(&disk_bytes(&path), SECRET));
        let store = RoomStore::open_with_keys(&path, &keys(2)).unwrap();
        assert_eq!(raw_texts(&store), first);
    }

    #[tokio::test]
    async fn a_message_copied_to_another_row_does_not_open_and_is_skipped() {
        let store = RoomStore::open_in_memory_with_keys(&keys(3)).unwrap();
        let room_id = conversation(&store).await;
        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute(
                    "UPDATE room_messages SET text = (SELECT text FROM room_messages WHERE role = 'user')
                     WHERE role = 'assistant'",
                    [],
                )
                .unwrap();
        }
        let snapshot = store.snapshot(&room_id).await.unwrap().unwrap();
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(snapshot.messages[0].text, SECRET);
        assert_eq!(store.seal_health()["unreadable_rows"], 1);
    }

    /// A room whose guest turn started a run that never finished.
    async fn interrupted(store: &RoomStore) -> (String, String) {
        let created = store
            .create_room(CreateRoomInput {
                model_id: "model-a".to_owned(),
                share_origin: "http://192.168.1.20:7980".to_owned(),
            })
            .await
            .unwrap();
        let session = store.exchange_invite(&created.invite, GUEST).await.unwrap();
        let accepted = store
            .append_user_turn(
                &session,
                SubmitTurnInput {
                    text: SECRET.to_owned(),
                    idempotency_key: None,
                },
            )
            .await
            .unwrap();
        store
            .append_delta(&created.room.id, &accepted.run.run_id, "quokka-")
            .await
            .unwrap();
        (created.room.id, accepted.run.run_id)
    }

    #[tokio::test]
    async fn recovery_on_a_store_without_its_key_does_not_fail_startup() {
        let path = db_path();
        let (room_id, run_id) = {
            let store = RoomStore::open_with_keys(&path, &keys(6)).unwrap();
            interrupted(&store).await
        };
        // Reopened under Core with no delivered key: sealed data, no key.
        let keyless = RoomStore::open_with_keys(&path, &PluginKeys::compat()).unwrap();
        assert_eq!(keyless.seal_health()["state"], "unhealthy");
        assert_eq!(keyless.recover_running().await.unwrap(), 1);
        // The snapshot still renders: the sealed run text is blanked and counted.
        let snapshot = keyless.snapshot(&room_id).await.unwrap().unwrap();
        let run = snapshot.current_run.expect("the failed run is shown");
        assert_eq!(run.run_id, run_id);
        assert_eq!(run.partial_text, "");
        assert_eq!(run.error_message, None);
        assert!(keyless.seal_health()["unreadable_rows"].as_u64().unwrap() >= 1);
        drop(keyless);
        // Back under the right key the run reads as recovered.
        let back = RoomStore::open_with_keys(&path, &keys(6)).unwrap();
        let run = back
            .snapshot(&room_id)
            .await
            .unwrap()
            .unwrap()
            .current_run
            .unwrap();
        assert_eq!(run.error_code.as_deref(), Some("nodeRestarted"));
        assert_eq!(run.partial_text, "quokka-");
    }

    #[tokio::test]
    async fn an_unreadable_current_run_does_not_hide_the_room() {
        let store = RoomStore::open_in_memory_with_keys(&keys(7)).unwrap();
        let (room_id, run_id) = interrupted(&store).await;
        {
            let connection = store.connection.lock().unwrap();
            // A partial sealed for another row (copied ciphertext).
            connection
                .execute(
                    "UPDATE room_runs SET partial_text = (SELECT text FROM room_messages LIMIT 1)
                     WHERE id = ?1",
                    params![run_id],
                )
                .unwrap();
        }
        let snapshot = store.snapshot(&room_id).await.unwrap().unwrap();
        assert_eq!(snapshot.messages[0].text, SECRET);
        assert_eq!(snapshot.current_run.unwrap().partial_text, "");
        assert_eq!(store.seal_health()["unreadable_rows"], 1);
    }

    #[tokio::test]
    async fn another_key_refuses_writes_and_keeps_data() {
        let path = db_path();
        let room_id = {
            let store = RoomStore::open_with_keys(&path, &keys(4)).unwrap();
            conversation(&store).await
        };
        let wrong = RoomStore::open_with_keys(&path, &keys(5)).unwrap();
        assert_eq!(wrong.seal_health()["state"], "unhealthy");
        assert!(wrong
            .create_room(CreateRoomInput {
                model_id: "model-a".to_owned(),
                share_origin: "http://192.168.1.20:7980".to_owned(),
            })
            .await
            .is_err());
        drop(wrong);
        let back = RoomStore::open_with_keys(&path, &keys(4)).unwrap();
        assert_eq!(
            back.snapshot(&room_id).await.unwrap().unwrap().messages[1].text,
            ANSWER
        );
    }
}
