use std::time::Duration;

use axum::{
    body::Body,
    extract::Json,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use ryu_app_events::{ApplicationRoomPublisher, ModelStreamClient};
use ryu_rooms::{
    inference::RoomRuntime,
    realtime::RoomEventHub,
    store::{CreateRoomInput, RoomStore, SubmitTurnInput},
};
use serde_json::{json, Value};

async fn fake_model(Json(body): Json<Value>) -> Response {
    let request_id = body["requestId"].as_str().expect("request id");
    let model = body["model"].as_str().expect("model");
    let content = if model == "broken" {
        "partial"
    } else {
        "hello world"
    };
    let mut frames = format!(
        "data: {}\n\n",
        json!({
            "type": "textDelta",
            "requestId": request_id,
            "delta": content
        })
    );
    if model != "broken" {
        frames.push_str(&format!(
            "data: {}\n\n",
            json!({ "type": "completed", "requestId": request_id })
        ));
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from(frames),
    )
        .into_response()
}

async fn fake_model_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("fake model listener");
    let address = listener.local_addr().expect("fake model address");
    let app = Router::new().route("/model", post(fake_model));
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("fake model server");
    });
    (format!("http://{address}/model"), handle)
}

async fn wait_for_status(store: &RoomStore, room_id: &str, expected: &str) -> Value {
    for _ in 0..40 {
        let snapshot = store
            .snapshot(room_id)
            .await
            .expect("snapshot")
            .expect("room");
        let value = serde_json::to_value(&snapshot).expect("snapshot json");
        if value["status"] == expected {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("room did not reach status {expected}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_model_stream_publishes_deltas_and_one_assistant_message() {
    let (endpoint, server) = fake_model_server().await;
    let store = RoomStore::open_in_memory().expect("store");
    let created = store
        .create_room(CreateRoomInput {
            model_id: "model-a".to_owned(),
            share_origin: "http://node.example".to_owned(),
        })
        .await
        .expect("room");
    let session = store
        .exchange_invite(&created.invite, "Phone")
        .await
        .expect("session");
    let events = RoomEventHub::new();
    let mut receiver = events.subscribe(&created.room.id).await;
    let runtime = RoomRuntime::new(
        store.clone(),
        ModelStreamClient::with_endpoint(
            "@ryu/rooms",
            reqwest::Client::new(),
            endpoint,
            "test-token",
        ),
        ApplicationRoomPublisher::disabled("@ryu/rooms"),
        events,
    );

    runtime
        .submit_turn(
            &session,
            SubmitTurnInput {
                text: "say hello".to_owned(),
                idempotency_key: Some("turn-1".to_owned()),
            },
        )
        .await
        .expect("accepted turn");

    let mut names = Vec::new();
    for _ in 0..3 {
        let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("room event timeout")
            .expect("room event");
        names.push(event.name);
    }
    assert_eq!(names, ["turn.accepted", "turn.delta", "turn.completed"]);

    let snapshot = wait_for_status(&store, &created.room.id, "idle").await;
    assert_eq!(snapshot["messages"].as_array().expect("messages").len(), 2);
    assert_eq!(
        snapshot["messages"][1]["text"], "hello world",
        "assistant output is persisted exactly once"
    );
    assert_eq!(snapshot["messages"][1]["role"], "assistant");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stream_without_terminal_event_fails_without_completed_assistant_output() {
    let (endpoint, server) = fake_model_server().await;
    let store = RoomStore::open_in_memory().expect("store");
    let created = store
        .create_room(CreateRoomInput {
            model_id: "broken".to_owned(),
            share_origin: "http://node.example".to_owned(),
        })
        .await
        .expect("room");
    let session = store
        .exchange_invite(&created.invite, "Phone")
        .await
        .expect("session");
    let events = RoomEventHub::new();
    let runtime = RoomRuntime::new(
        store.clone(),
        ModelStreamClient::with_endpoint(
            "@ryu/rooms",
            reqwest::Client::new(),
            endpoint,
            "test-token",
        ),
        ApplicationRoomPublisher::disabled("@ryu/rooms"),
        events,
    );

    runtime
        .submit_turn(
            &session,
            SubmitTurnInput {
                text: "break".to_owned(),
                idempotency_key: None,
            },
        )
        .await
        .expect("accepted turn");

    let snapshot = wait_for_status(&store, &created.room.id, "failed").await;
    assert_eq!(snapshot["currentRun"]["status"], "failed");
    assert_eq!(snapshot["currentRun"]["partialText"], "partial");
    assert_eq!(
        snapshot["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .filter(|message| message["role"] == "assistant")
            .count(),
        0,
        "partial text must not be presented as a completed assistant message"
    );
    server.abort();
}
