use std::convert::Infallible;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    guest_html::GUEST_HTML,
    model::RoomSession,
    store::{CreateRoomInput, RoomStore, StoreError, SubmitTurnInput},
    RoomRuntime,
};

const SESSION_COOKIE: &str = "ryu_rooms_session";
const SESSION_MAX_AGE_SECS: u64 = 12 * 60 * 60;

#[derive(Clone)]
pub struct RoomsCtx {
    pub store: RoomStore,
    pub runtime: RoomRuntime,
}

impl RoomsCtx {
    pub fn new(store: RoomStore, runtime: RoomRuntime) -> Self {
        Self { store, runtime }
    }
}

pub fn routes(ctx: RoomsCtx) -> Router<()> {
    Router::new()
        .route("/", get(list_rooms).post(create_room))
        .route("/:room_id", get(get_room))
        .route("/:room_id/turns", post(host_turn))
        .route("/:room_id/stop", post(stop_run))
        .route("/:room_id/close", post(close_room))
        .route("/:room_id/invite", post(issue_invite))
        .route("/:room_id/invite/revoke", post(revoke_invite))
        .route("/guest", get(guest_html))
        .route("/guest/exchange", post(exchange_invite))
        .route("/guest/snapshot", get(guest_snapshot))
        .route("/guest/events", get(guest_events))
        .route("/guest/turns", post(guest_turn))
        .with_state(ctx)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRoomBody {
    model_id: String,
    share_origin: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnBody {
    text: String,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StopBody {
    run_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExchangeBody {
    invite: String,
    display_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    code: String,
    message: String,
    request_id: String,
}

async fn list_rooms(State(ctx): State<RoomsCtx>, headers: HeaderMap) -> Response {
    match ctx.store.list_rooms().await {
        Ok(rooms) => Json(json!({ "rooms": rooms })).into_response(),
        Err(error) => store_error(error, request_id(&headers)),
    }
}

async fn create_room(
    State(ctx): State<RoomsCtx>,
    headers: HeaderMap,
    Json(body): Json<CreateRoomBody>,
) -> Response {
    let request_id = request_id(&headers);
    match ctx
        .store
        .create_room(CreateRoomInput {
            model_id: body.model_id,
            share_origin: body.share_origin,
        })
        .await
    {
        Ok(created) => {
            let join_url = format!(
                "{}/api/rooms/guest#invite={}",
                created.share_origin.trim_end_matches('/'),
                created.invite
            );
            (
                StatusCode::CREATED,
                Json(json!({ "room": created.room, "joinUrl": join_url })),
            )
                .into_response()
        }
        Err(error) => store_error(error, request_id),
    }
}

async fn get_room(
    State(ctx): State<RoomsCtx>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
) -> Response {
    match ctx.store.snapshot(&room_id).await {
        Ok(Some(room)) => Json(room).into_response(),
        Ok(None) => store_error(StoreError::NotFound, request_id(&headers)),
        Err(error) => store_error(error, request_id(&headers)),
    }
}

async fn host_turn(
    State(ctx): State<RoomsCtx>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
    Json(body): Json<TurnBody>,
) -> Response {
    let request_id = request_id(&headers);
    let session = match ctx.store.host_session(&room_id).await {
        Ok(session) => session,
        Err(error) => return store_error(error, request_id),
    };
    submit_turn(&ctx, &session, body, request_id).await
}

async fn guest_turn(
    State(ctx): State<RoomsCtx>,
    headers: HeaderMap,
    Json(body): Json<TurnBody>,
) -> Response {
    let request_id = request_id(&headers);
    let session = match session_from_headers(&ctx.store, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    submit_turn(&ctx, &session, body, request_id).await
}

async fn submit_turn(
    ctx: &RoomsCtx,
    session: &RoomSession,
    body: TurnBody,
    request_id: String,
) -> Response {
    match ctx
        .runtime
        .submit_turn(
            session,
            SubmitTurnInput {
                text: body.text,
                idempotency_key: body.idempotency_key,
            },
        )
        .await
    {
        Ok(accepted) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "run": accepted.run,
                "message": accepted.user_message,
                "snapshot": accepted.snapshot,
                "requestId": request_id,
            })),
        )
            .into_response(),
        Err(error) => store_error(error, request_id),
    }
}

async fn stop_run(
    State(ctx): State<RoomsCtx>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
    Json(body): Json<StopBody>,
) -> Response {
    let request_id = request_id(&headers);
    match ctx.runtime.stop_run(&room_id, &body.run_id).await {
        Ok(snapshot) => {
            Json(json!({ "snapshot": snapshot, "requestId": request_id })).into_response()
        }
        Err(error) => store_error(error, request_id),
    }
}

async fn close_room(
    State(ctx): State<RoomsCtx>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
) -> Response {
    let request_id = request_id(&headers);
    match ctx.runtime.close_room(&room_id).await {
        Ok(snapshot) => {
            Json(json!({ "snapshot": snapshot, "requestId": request_id })).into_response()
        }
        Err(error) => store_error(error, request_id),
    }
}

async fn revoke_invite(
    State(ctx): State<RoomsCtx>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
) -> Response {
    let request_id = request_id(&headers);
    match ctx.store.revoke_invite(&room_id).await {
        Ok(()) => Json(json!({ "revoked": true, "requestId": request_id })).into_response(),
        Err(error) => store_error(error, request_id),
    }
}

async fn issue_invite(
    State(ctx): State<RoomsCtx>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
) -> Response {
    let request_id = request_id(&headers);
    match ctx.store.issue_invite(&room_id).await {
        Ok(invite) => (
            StatusCode::OK,
            Json(json!({
                "joinUrl": format!("{}/api/rooms/guest#invite={}", invite.share_origin, invite.invite),
                "requestId": request_id,
            })),
        )
            .into_response(),
        Err(error) => store_error(error, request_id),
    }
}

async fn guest_html() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        GUEST_HTML,
    )
        .into_response()
}

async fn exchange_invite(
    State(ctx): State<RoomsCtx>,
    headers: HeaderMap,
    Json(body): Json<ExchangeBody>,
) -> Response {
    let request_id = request_id(&headers);
    let session = match ctx
        .store
        .exchange_invite(&body.invite, &body.display_name)
        .await
    {
        Ok(session) => session,
        Err(error) => return store_error(error, request_id),
    };
    let snapshot = match ctx.store.snapshot(&session.room_id).await {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => return store_error(StoreError::NotFound, request_id),
        Err(error) => return store_error(error, request_id),
    };
    let cookie = session_cookie(&session.secret, is_https_request(&headers));
    let mut response = Json(json!({
        "room": snapshot,
        "participantId": session.participant_id,
        "displayName": session.display_name,
        "role": session.role,
        "requestId": request_id,
    }))
    .into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    response
}

async fn guest_snapshot(State(ctx): State<RoomsCtx>, headers: HeaderMap) -> Response {
    let session = match session_from_headers(&ctx.store, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    match ctx.store.snapshot(&session.room_id).await {
        Ok(Some(snapshot)) => Json(snapshot).into_response(),
        Ok(None) => store_error(StoreError::NotFound, request_id(&headers)),
        Err(error) => store_error(error, request_id(&headers)),
    }
}

async fn guest_events(State(ctx): State<RoomsCtx>, headers: HeaderMap) -> Response {
    let session = match session_from_headers(&ctx.store, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let room_id = session.room_id.clone();
    let snapshot = match ctx.store.snapshot(&room_id).await {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => return store_error(StoreError::NotFound, request_id(&headers)),
        Err(error) => return store_error(error, request_id(&headers)),
    };
    let mut receiver = ctx.runtime.events().subscribe(&room_id).await;
    let store = ctx.store.clone();
    let stream = async_stream::stream! {
        yield Ok::<Event, Infallible>(sse_event("snapshot", serde_json::to_value(snapshot).unwrap_or(Value::Null)));
        loop {
            match receiver.recv().await {
                Ok(event) => yield Ok(sse_event(&event.name, event.data)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    match store.snapshot(&room_id).await {
                        Ok(Some(snapshot)) => yield Ok(sse_event("snapshot", serde_json::to_value(snapshot).unwrap_or(Value::Null))),
                        _ => break,
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn session_from_headers(
    store: &RoomStore,
    headers: &HeaderMap,
) -> Result<RoomSession, Response> {
    let Some(secret) = cookie_value(headers, SESSION_COOKIE) else {
        return Err(unauthorized_response(request_id(headers)));
    };
    match store.resolve_session(secret).await {
        Ok(Some(session)) => Ok(session),
        Ok(None) => Err(unauthorized_response(request_id(headers))),
        Err(error) => Err(store_error(error, request_id(headers))),
    }
}

fn store_error(error: StoreError, request_id: String) -> Response {
    let status = match &error {
        StoreError::Invalid(_) => StatusCode::BAD_REQUEST,
        StoreError::NotFound => StatusCode::NOT_FOUND,
        StoreError::RoomBusy | StoreError::Conflict => StatusCode::CONFLICT,
        StoreError::Closed => StatusCode::GONE,
        StoreError::Forbidden => StatusCode::FORBIDDEN,
        StoreError::InviteUnavailable => StatusCode::BAD_REQUEST,
        StoreError::Database(_) | StoreError::Lock => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        status,
        Json(ErrorBody {
            code: error.code().to_owned(),
            message: public_error_message(&error),
            request_id,
        }),
    )
        .into_response()
}

fn public_error_message(error: &StoreError) -> String {
    match error {
        StoreError::Invalid(message) => message.clone(),
        StoreError::RoomBusy => "another turn is already running".to_owned(),
        StoreError::Closed => "this room is closed".to_owned(),
        StoreError::InviteUnavailable => "the invite is invalid, expired, or revoked".to_owned(),
        StoreError::Conflict => "the request conflicts with the current room state".to_owned(),
        StoreError::NotFound => "room not found".to_owned(),
        StoreError::Forbidden => "permission denied".to_owned(),
        StoreError::Database(_) | StoreError::Lock => {
            "room storage is temporarily unavailable".to_owned()
        }
    }
}

fn unauthorized_response(request_id: String) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody {
            code: "unauthorized".to_owned(),
            message: "a valid guest session is required".to_owned(),
            request_id,
        }),
    )
        .into_response()
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.trim().is_empty()
                && value.len() <= 128
                && value.chars().all(|character| !character.is_control())
        })
        .map(str::to_owned)
        .unwrap_or_else(|| format!("req_{}", Uuid::new_v4().simple()))
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (cookie_name, cookie_value) = cookie.trim().split_once('=')?;
                (cookie_name == name && !cookie_value.is_empty()).then_some(cookie_value)
            })
        })
}

fn session_cookie(secret: &str, secure: bool) -> HeaderValue {
    let secure_suffix = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={secret}; Path=/api/rooms; Max-Age={SESSION_MAX_AGE_SECS}; HttpOnly; SameSite=Strict{secure_suffix}"
    ))
    .expect("generated session cookie is a valid header value")
}

fn is_https_request(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"))
        || headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.trim_start().starts_with("https://"))
}

fn sse_event(name: &str, data: Value) -> Event {
    Event::default()
        .event(name)
        .json_data(data)
        .unwrap_or_else(|_| {
            Event::default()
                .event("error")
                .data("stream encoding failed")
        })
}
