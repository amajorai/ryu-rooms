//! Stop-run and close-room over the real router. The run under test is
//! appended straight to the store so no model generation races the stop.

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use ryu_app_events::{ApplicationRoomPublisher, ModelStreamClient};
use ryu_rooms::{
    api::{routes, RoomsCtx},
    inference::RoomRuntime,
    realtime::RoomEventHub,
    store::{CreateRoomInput, RoomStore, SubmitTurnInput},
};
use serde_json::{json, Value};
use tower::ServiceExt;

fn ctx() -> (RoomsCtx, RoomStore) {
    let store = RoomStore::open_in_memory().expect("store");
    let runtime = RoomRuntime::new(
        store.clone(),
        ModelStreamClient::disabled("@ryu/rooms"),
        ApplicationRoomPublisher::disabled("@ryu/rooms"),
        RoomEventHub::new(),
    );
    (RoomsCtx::new(store.clone(), runtime), store)
}

async fn call(
    context: &RoomsCtx,
    method: &str,
    uri: &str,
    body: Option<Value>,
    request_id: Option<&str>,
    cookie: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    if let Some(request_id) = request_id {
        builder = builder.header("x-request-id", request_id);
    }
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let request_body = body.map_or_else(Body::empty, |value| Body::from(value.to_string()));
    let response = routes(context.clone())
        .oneshot(builder.body(request_body).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2_000_000)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A room with a guest whose turn is recorded as running (no generation task).
async fn room_with_running_run(store: &RoomStore) -> (String, String) {
    let created = store
        .create_room(CreateRoomInput {
            model_id: "mesh-model".to_owned(),
            share_origin: "http://node.example".to_owned(),
        })
        .await
        .expect("room");
    let session = store
        .exchange_invite(&created.invite, "Phone")
        .await
        .expect("session");
    let accepted = store
        .append_user_turn(
            &session,
            SubmitTurnInput {
                text: "hello".to_owned(),
                idempotency_key: Some("k1".to_owned()),
            },
        )
        .await
        .expect("turn");
    (created.room.id, accepted.run.run_id)
}

#[tokio::test]
async fn stopping_a_run_cancels_it_once_and_echoes_the_request_id() {
    let (context, store) = ctx();
    let (room_id, run_id) = room_with_running_run(&store).await;
    let stop = format!("/{room_id}/stop");
    let before = store.snapshot(&room_id).await.unwrap().unwrap();
    assert_eq!(
        before.current_run.unwrap().status,
        ryu_rooms::model::RunStatus::Running
    );

    let (status, body) = call(
        &context,
        "POST",
        &stop,
        Some(json!({ "runId": run_id })),
        Some("req-stop-1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["requestId"], "req-stop-1");
    // Canceled runs leave the snapshot; the room is idle again.
    assert_eq!(body["snapshot"]["currentRun"], Value::Null);
    assert_eq!(body["snapshot"]["status"], "idle");
    assert!(store
        .snapshot(&room_id)
        .await
        .unwrap()
        .unwrap()
        .current_run
        .is_none());

    // A finished run cannot be stopped again, and an unknown run maps to 404.
    for (run, request_id) in [
        (run_id.as_str(), "req-again"),
        ("run_unknown", "req-unknown"),
    ] {
        let (status, body) = call(
            &context,
            "POST",
            &stop,
            Some(json!({ "runId": run })),
            Some(request_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{run}");
        assert_eq!(body["code"], "roomNotFound");
        assert_eq!(body["requestId"], request_id);
    }

    // A malformed room id is a 400, never a store lookup.
    let (status, body) = call(
        &context,
        "POST",
        "/not-a-room/stop",
        Some(json!({ "runId": run_id })),
        Some("req-bad"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalidArgs");
    assert_eq!(body["requestId"], "req-bad");
}

#[tokio::test]
async fn closing_a_room_cancels_its_run_and_refuses_further_posts() {
    let (context, store) = ctx();
    let created = call(
        &context,
        "POST",
        "/",
        Some(json!({ "modelId": "mesh-model", "shareOrigin": "http://node.example" })),
        None,
        None,
    )
    .await
    .1;
    let room_id = created["room"]["id"].as_str().unwrap().to_owned();
    let invite = created["joinUrl"]
        .as_str()
        .and_then(|value| value.split_once("#invite=").map(|(_, invite)| invite))
        .unwrap()
        .to_owned();
    let session = store.exchange_invite(&invite, "Phone").await.unwrap();
    let cookie = format!("ryu_rooms_session={}", session.secret);
    let run_id = store
        .append_user_turn(
            &session,
            SubmitTurnInput {
                text: "hello".to_owned(),
                idempotency_key: None,
            },
        )
        .await
        .unwrap()
        .run
        .run_id;
    let (status, _) = call(
        &context,
        "GET",
        "/guest/snapshot",
        None,
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the guest session is live before close"
    );

    let (status, body) = call(
        &context,
        "POST",
        &format!("/{room_id}/close"),
        None,
        Some("req-close"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["requestId"], "req-close");
    assert_eq!(body["snapshot"]["status"], "closed");
    assert_eq!(
        body["snapshot"]["currentRun"],
        Value::Null,
        "the run was canceled"
    );
    assert!(body["snapshot"]["participants"]
        .as_array()
        .unwrap()
        .iter()
        .all(|participant| participant["online"] == false));

    // The host can no longer post a turn or mint an invite.
    let (status, body) = call(
        &context,
        "POST",
        &format!("/{room_id}/turns"),
        Some(json!({ "text": "after close" })),
        Some("req-turn"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["code"], "roomClosed");
    assert_eq!(body["requestId"], "req-turn");
    let (status, _) = call(
        &context,
        "POST",
        &format!("/{room_id}/invite"),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::GONE);

    // The guest's session was revoked with the room.
    let (status, _) = call(
        &context,
        "POST",
        "/guest/turns",
        Some(json!({ "text": "guest after close" })),
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let stored = store.snapshot(&room_id).await.unwrap().unwrap();
    assert_eq!(stored.messages.len(), 1, "no post landed after close");

    // The queued run was canceled by the close, so it is no longer stoppable.
    let (status, _) = call(
        &context,
        "POST",
        &format!("/{room_id}/stop"),
        Some(json!({ "runId": run_id })),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Closing twice is a 404, not a silent success.
    let (status, body) = call(
        &context,
        "POST",
        &format!("/{room_id}/close"),
        None,
        Some("req-close-2"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["requestId"], "req-close-2");
}
