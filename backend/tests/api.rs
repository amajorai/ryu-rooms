use std::time::Duration;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use futures_util::StreamExt;
use ryu_app_events::{ApplicationRoomPublisher, ModelStreamClient};
use ryu_rooms::{
    api::{routes, RoomsCtx},
    inference::RoomRuntime,
    realtime::RoomEventHub,
    store::RoomStore,
};
use serde_json::{json, Value};
use tower::ServiceExt;

fn ctx() -> RoomsCtx {
    let store = RoomStore::open_in_memory().expect("store");
    let runtime = RoomRuntime::new(
        store.clone(),
        ModelStreamClient::disabled("@ryu/rooms"),
        ApplicationRoomPublisher::disabled("@ryu/rooms"),
        RoomEventHub::new(),
    );
    RoomsCtx::new(store, runtime)
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 2_000_000)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("json response")
}

async fn request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let request_body = body.map_or_else(Body::empty, |value| Body::from(value.to_string()));
    app.oneshot(builder.body(request_body).expect("request"))
        .await
        .expect("response")
}

#[tokio::test]
async fn creates_room_with_fragment_invite_and_no_secret_snapshot_field() {
    let response = request(
        routes(ctx()),
        "POST",
        "/",
        Some(json!({
            "modelId": "mesh-model",
            "shareOrigin": "https://node.example"
        })),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    let join_url = body["joinUrl"].as_str().expect("join url");
    assert!(join_url.starts_with("https://node.example/api/rooms/guest#invite="));
    assert!(!join_url.contains('?'));
    assert!(body["room"].get("nodeToken").is_none());
    assert!(body["room"].get("providerKey").is_none());
}

#[tokio::test]
async fn guest_exchange_sets_strict_http_only_session_and_snapshot_is_session_bound() {
    let context = ctx();
    let created = request(
        routes(context.clone()),
        "POST",
        "/",
        Some(json!({
            "modelId": "mesh-model",
            "shareOrigin": "https://node.example"
        })),
        None,
    )
    .await;
    let created = body_json(created).await;
    let invite = created["joinUrl"]
        .as_str()
        .and_then(|value| value.split_once("#invite=").map(|(_, invite)| invite))
        .expect("fragment invite");

    let unauthorized = request(
        routes(context.clone()),
        "GET",
        "/guest/snapshot",
        None,
        None,
    )
    .await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let exchange = routes(context.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/guest/exchange")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://node.example")
                .body(Body::from(
                    json!({ "invite": invite, "displayName": "Phone" }).to_string(),
                ))
                .expect("exchange request"),
        )
        .await
        .expect("exchange response");
    assert_eq!(exchange.status(), StatusCode::OK);
    let cookie = exchange
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie")
        .to_str()
        .expect("cookie header")
        .to_owned();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("Path=/api/rooms"));
    assert!(!cookie.contains(invite));
    let cookie_pair = cookie.split(';').next().expect("cookie pair");

    let snapshot = request(
        routes(context),
        "GET",
        "/guest/snapshot",
        None,
        Some(cookie_pair),
    )
    .await;
    assert_eq!(snapshot.status(), StatusCode::OK);
    let snapshot = body_json(snapshot).await;
    assert_eq!(snapshot["modelId"], "mesh-model");
    assert!(snapshot.get("session").is_none());
    assert!(snapshot.get("invite").is_none());
}

#[tokio::test]
async fn invalid_invite_is_generic_and_guest_html_is_data_free() {
    let context = ctx();
    let invalid = "not-the-invite";
    let response = request(
        routes(context.clone()),
        "POST",
        "/guest/exchange",
        Some(json!({ "invite": invalid, "displayName": "Phone" })),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), 100_000)
        .await
        .expect("error body");
    let text = String::from_utf8(bytes.to_vec()).expect("utf8");
    assert!(text.contains("inviteUnavailable"));
    assert!(!text.contains(invalid));

    let html = request(routes(context), "GET", "/guest", None, None).await;
    assert_eq!(html.status(), StatusCode::OK);
    assert_eq!(
        html.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    let html = to_bytes(html.into_body(), 2_000_000)
        .await
        .expect("html body");
    let html = String::from_utf8(html.to_vec()).expect("html utf8");
    assert!(html.contains("Ryu Room"));
    assert!(!html.contains("not-the-invite"));
}

#[tokio::test]
async fn guest_events_send_snapshot_before_waiting_for_live_changes() {
    let context = ctx();
    let created = request(
        routes(context.clone()),
        "POST",
        "/",
        Some(json!({
            "modelId": "mesh-model",
            "shareOrigin": "http://node.example"
        })),
        None,
    )
    .await;
    let created = body_json(created).await;
    let invite = created["joinUrl"]
        .as_str()
        .and_then(|value| value.split_once("#invite=").map(|(_, invite)| invite))
        .expect("fragment invite");
    let exchange = request(
        routes(context.clone()),
        "POST",
        "/guest/exchange",
        Some(json!({ "invite": invite, "displayName": "Browser" })),
        None,
    )
    .await;
    let cookie = exchange
        .headers()
        .get(header::SET_COOKIE)
        .expect("cookie")
        .to_str()
        .expect("cookie utf8")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();

    let response = request(routes(context), "GET", "/guest/events", None, Some(&cookie)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    let first = tokio::time::timeout(Duration::from_secs(1), body.next())
        .await
        .expect("snapshot event timeout")
        .expect("snapshot chunk")
        .expect("snapshot body");
    let first = String::from_utf8(first.to_vec()).expect("sse utf8");
    assert!(first.contains("event: snapshot"));
    assert!(first.contains("mesh-model"));
}

#[tokio::test]
async fn guest_turn_is_accepted_and_second_turn_is_busy_while_run_is_running() {
    let context = ctx();
    let created = request(
        routes(context.clone()),
        "POST",
        "/",
        Some(json!({
            "modelId": "mesh-model",
            "shareOrigin": "http://node.example"
        })),
        None,
    )
    .await;
    let created = body_json(created).await;
    let invite = created["joinUrl"]
        .as_str()
        .and_then(|value| value.split_once("#invite=").map(|(_, invite)| invite))
        .expect("fragment invite");
    let exchange = request(
        routes(context.clone()),
        "POST",
        "/guest/exchange",
        Some(json!({ "invite": invite, "displayName": "Browser" })),
        None,
    )
    .await;
    let cookie = exchange
        .headers()
        .get(header::SET_COOKIE)
        .expect("cookie")
        .to_str()
        .expect("cookie utf8")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();

    let first = request(
        routes(context.clone()),
        "POST",
        "/guest/turns",
        Some(json!({ "text": "first", "idempotencyKey": "first" })),
        Some(&cookie),
    )
    .await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);

    let second = request(
        routes(context),
        "POST",
        "/guest/turns",
        Some(json!({ "text": "second" })),
        Some(&cookie),
    )
    .await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let body = body_json(second).await;
    assert_eq!(body["code"], "roomBusy");
}
