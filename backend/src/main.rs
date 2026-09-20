use std::net::{Ipv4Addr, SocketAddr};

use axum::{
    extract::Request,
    http::{header::AUTHORIZATION, StatusCode},
    middleware::{from_fn, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;

use ryu_app_events::{ApplicationRoomPublisher, ModelStreamClient};
use ryu_rooms::{api, inference::RoomRuntime, paths, realtime::RoomEventHub, store::RoomStore};

const DEFAULT_PORT: u16 = 8024;
const PLUGIN_ID: &str = "@ryu/rooms";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let port = std::env::var("RYU_ROOMS_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    let token = std::env::var("RYU_EXT_TOKEN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if token.is_none() {
        tracing::warn!(
            "ryu-rooms: no RYU_EXT_TOKEN set; every /api/rooms route is fail-closed until Core spawns this sidecar"
        );
    }

    let store = RoomStore::open(paths::database_path())?;
    let recovered = store.recover_running().await?;
    if recovered > 0 {
        tracing::warn!(
            runs = recovered,
            "rooms: marked interrupted generations as failed"
        );
    }

    let plugin_id = std::env::var("RYU_EXT_PLUGIN_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| PLUGIN_ID.to_owned());
    let events = RoomEventHub::new();
    let runtime = RoomRuntime::new(
        store.clone(),
        ModelStreamClient::from_env(plugin_id.clone()),
        ApplicationRoomPublisher::from_env(plugin_id),
        events,
    );
    let ctx = api::RoomsCtx::new(store.clone(), runtime);
    let expected_token = token.clone();
    let protected = Router::new()
        .nest("/api/rooms", api::routes(ctx))
        .layer(from_fn(move |request: Request, next: Next| {
            let expected = expected_token.clone();
            async move { bearer_gate(expected.as_deref(), request, next).await }
        }));
    let app = Router::new().route("/health", get(health)).merge(protected);

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "ryu-rooms listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn bearer_gate(expected: Option<&str>, request: Request, next: Next) -> Response {
    let provided = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if ryu_sidecar_runtime::token_ok(provided, expected) {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "code": "unauthorized",
            "message": "the Rooms sidecar requires a Core-issued request"
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{header::AUTHORIZATION, Request, StatusCode},
        middleware::{from_fn, Next},
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    use super::bearer_gate;

    #[test]
    fn sidecar_bearer_gate_fails_closed() {
        assert!(!ryu_sidecar_runtime::token_ok(Some("anything"), None));
        assert!(!ryu_sidecar_runtime::token_ok(None, Some("secret")));
        assert!(ryu_sidecar_runtime::token_ok(
            Some("secret"),
            Some("secret")
        ));
    }

    #[tokio::test]
    async fn http_shell_rejects_missing_and_wrong_bearers() {
        let app = Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(from_fn(|request: Request<Body>, next: Next| async move {
                bearer_gate(Some("secret"), request, next).await
            }));
        let missing = app
            .clone()
            .oneshot(Request::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .expect("missing response");
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        let wrong = app
            .oneshot(
                Request::builder()
                    .uri("/x")
                    .header(AUTHORIZATION, "Bearer wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("wrong response");
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    }
}
