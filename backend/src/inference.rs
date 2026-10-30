use std::{collections::HashMap, sync::Arc};

use futures_util::StreamExt;
use ryu_app_events::{
    ApplicationRoomPublisher, ModelStreamClient, ModelStreamError, ModelStreamEvent,
    ModelStreamMessage, ModelStreamRequest,
};
use serde_json::json;
use tokio::sync::{watch, Mutex};

use crate::{
    model::{MessageRole, RoomEvent, RoomSession, RoomSnapshot, RunStatus},
    realtime::RoomEventHub,
    store::{AcceptedRun, RoomStore, RunTerminal, StoreError, SubmitTurnInput},
};

const MODEL_MAX_TOKENS: u32 = 2048;
const MODEL_TEMPERATURE: f32 = 0.2;

#[derive(Clone)]
struct ActiveRun {
    run_id: String,
    cancel: watch::Sender<bool>,
}

/// Coordinates the one active generation allowed for each room and projects its
/// durable changes to both guest SSE subscribers and Core's application-room
/// realtime channel.
#[derive(Clone)]
pub struct RoomRuntime {
    store: RoomStore,
    model: ModelStreamClient,
    publisher: ApplicationRoomPublisher,
    events: RoomEventHub,
    active: Arc<Mutex<HashMap<String, ActiveRun>>>,
}

impl RoomRuntime {
    pub fn new(
        store: RoomStore,
        model: ModelStreamClient,
        publisher: ApplicationRoomPublisher,
        events: RoomEventHub,
    ) -> Self {
        Self {
            store,
            model,
            publisher,
            events,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[must_use]
    pub fn events(&self) -> RoomEventHub {
        self.events.clone()
    }

    pub async fn submit_turn(
        &self,
        session: &RoomSession,
        input: SubmitTurnInput,
    ) -> Result<AcceptedRun, StoreError> {
        let accepted = self.store.append_user_turn(session, input).await?;
        let room_id = accepted.snapshot.id.clone();
        self.publish(
            &room_id,
            "turn.accepted",
            json!({
                "run": accepted.run.clone(),
                "message": accepted.user_message.clone(),
                "snapshot": accepted.snapshot.clone(),
            }),
        )
        .await;
        if accepted.run.status == RunStatus::Running {
            self.start_run(accepted.clone()).await;
        }
        Ok(accepted)
    }

    pub async fn stop_run(&self, room_id: &str, run_id: &str) -> Result<RoomSnapshot, StoreError> {
        let cancel = {
            let active = self.active.lock().await;
            active
                .get(room_id)
                .filter(|run| run.run_id == run_id)
                .map(|run| run.cancel.clone())
        };
        if let Some(cancel) = cancel {
            let _ = cancel.send(true);
        }
        self.store.stop_run(room_id, run_id).await?;
        self.remove_active(room_id, run_id).await;
        let snapshot = self.snapshot(room_id).await?;
        self.publish(
            room_id,
            "turn.canceled",
            json!({ "runId": run_id, "snapshot": snapshot }),
        )
        .await;
        Ok(snapshot)
    }

    pub async fn close_room(&self, room_id: &str) -> Result<RoomSnapshot, StoreError> {
        let current = self.snapshot(room_id).await?;
        if let Some(run) = current.current_run.as_ref() {
            if matches!(run.status, RunStatus::Queued | RunStatus::Running) {
                let _ = self.stop_run(room_id, &run.run_id).await;
            }
        }
        self.store.close_room(room_id).await?;
        self.remove_active_for_room(room_id).await;
        let snapshot = self.snapshot(room_id).await?;
        self.publish(room_id, "room.closed", json!({ "snapshot": snapshot }))
            .await;
        Ok(snapshot)
    }

    async fn start_run(&self, accepted: AcceptedRun) {
        let room_id = accepted.snapshot.id.clone();
        let run_id = accepted.run.run_id.clone();
        let (cancel, cancel_rx) = watch::channel(false);
        {
            let mut active = self.active.lock().await;
            if active.contains_key(&room_id) {
                return;
            }
            active.insert(
                room_id.clone(),
                ActiveRun {
                    run_id: run_id.clone(),
                    cancel,
                },
            );
        }

        let runtime = self.clone();
        tokio::spawn(async move {
            runtime.run_generation(accepted, cancel_rx).await;
            runtime.remove_active(&room_id, &run_id).await;
        });
    }

    async fn run_generation(&self, accepted: AcceptedRun, mut cancel: watch::Receiver<bool>) {
        let room_id = accepted.snapshot.id.clone();
        let run_id = accepted.run.run_id.clone();
        let request_id = accepted.run.request_id.clone();
        let request = ModelStreamRequest {
            messages: accepted
                .snapshot
                .messages
                .iter()
                .map(|message| ModelStreamMessage {
                    role: match message.role {
                        MessageRole::User => "user".to_owned(),
                        MessageRole::Assistant => "assistant".to_owned(),
                    },
                    content: message.text.clone(),
                })
                .collect(),
            model: accepted.snapshot.model_id.clone(),
            provider: Some("local".to_owned()),
            request_id: request_id.clone(),
            max_tokens: Some(MODEL_MAX_TOKENS),
            temperature: Some(MODEL_TEMPERATURE),
        };

        let mut stream = match self.model.stream(request).await {
            Ok(stream) => stream,
            Err(error) => {
                self.fail_generation(&room_id, &run_id, &request_id, model_error_code(&error))
                    .await;
                return;
            }
        };
        let mut partial = String::new();

        loop {
            let next = tokio::select! {
                changed = cancel.changed() => {
                    if changed.is_ok() && *cancel.borrow() { return; }
                    continue;
                }
                event = stream.next() => event,
            };
            let Some(event) = next else {
                self.fail_generation(&room_id, &run_id, &request_id, "modelStreamEnded")
                    .await;
                return;
            };
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    if *cancel.borrow() {
                        return;
                    }
                    self.fail_generation(&room_id, &run_id, &request_id, model_error_code(&error))
                        .await;
                    return;
                }
            };
            match event {
                ModelStreamEvent::TextDelta {
                    request_id: event_request_id,
                    delta,
                } => {
                    if event_request_id != request_id {
                        self.fail_generation(&room_id, &run_id, &request_id, "modelProtocol")
                            .await;
                        return;
                    }
                    if delta.is_empty() {
                        continue;
                    }
                    partial.push_str(&delta);
                    if self
                        .store
                        .append_delta(&room_id, &run_id, &delta)
                        .await
                        .is_err()
                    {
                        if *cancel.borrow() {
                            return;
                        }
                        self.fail_generation(&room_id, &run_id, &request_id, "outputRejected")
                            .await;
                        return;
                    }
                    self.publish(
                        &room_id,
                        "turn.delta",
                        json!({
                            "runId": run_id,
                            "requestId": request_id,
                            "delta": delta,
                            "partialText": partial,
                        }),
                    )
                    .await;
                }
                ModelStreamEvent::Completed {
                    request_id: event_request_id,
                } => {
                    if event_request_id != request_id {
                        self.fail_generation(&room_id, &run_id, &request_id, "modelProtocol")
                            .await;
                        return;
                    }
                    if *cancel.borrow() {
                        return;
                    }
                    match self
                        .store
                        .finish_run(
                            &room_id,
                            &run_id,
                            RunTerminal {
                                status: RunStatus::Completed,
                                error_code: None,
                                error_message: None,
                            },
                        )
                        .await
                    {
                        Ok(snapshot) => {
                            self.publish(
                                &room_id,
                                "turn.completed",
                                json!({
                                    "runId": run_id,
                                    "requestId": request_id,
                                    "snapshot": snapshot,
                                }),
                            )
                            .await;
                        }
                        Err(_) => {
                            self.fail_generation(
                                &room_id,
                                &run_id,
                                &request_id,
                                "storeUnavailable",
                            )
                            .await;
                        }
                    }
                    return;
                }
                ModelStreamEvent::Failed {
                    request_id: event_request_id,
                    ..
                } => {
                    if event_request_id != request_id {
                        self.fail_generation(&room_id, &run_id, &request_id, "modelProtocol")
                            .await;
                    } else if !*cancel.borrow() {
                        self.fail_generation(&room_id, &run_id, &request_id, "modelFailed")
                            .await;
                    }
                    return;
                }
            }
        }
    }

    async fn fail_generation(&self, room_id: &str, run_id: &str, request_id: &str, code: &str) {
        let snapshot = match self
            .store
            .finish_run(
                room_id,
                run_id,
                RunTerminal {
                    status: RunStatus::Failed,
                    error_code: Some(code.to_owned()),
                    error_message: Some("the local model could not complete this turn".to_owned()),
                },
            )
            .await
        {
            Ok(snapshot) => snapshot,
            Err(_) => return,
        };
        self.publish(
            room_id,
            "turn.failed",
            json!({
                "runId": run_id,
                "requestId": request_id,
                "errorCode": code,
                "snapshot": snapshot,
            }),
        )
        .await;
    }

    async fn snapshot(&self, room_id: &str) -> Result<RoomSnapshot, StoreError> {
        self.store
            .snapshot(room_id)
            .await?
            .ok_or(StoreError::NotFound)
    }

    async fn publish(&self, room_id: &str, name: &str, data: serde_json::Value) {
        let listeners = self
            .events
            .publish(
                room_id,
                RoomEvent {
                    name: name.to_owned(),
                    data: data.clone(),
                },
            )
            .await;
        self.publisher
            .publish_best_effort(room_id, name, data)
            .await;
        tracing::debug!(room_id, event = name, listeners, "rooms event published");
    }

    async fn remove_active(&self, room_id: &str, run_id: &str) {
        let mut active = self.active.lock().await;
        if active.get(room_id).is_some_and(|run| run.run_id == run_id) {
            active.remove(room_id);
        }
    }

    async fn remove_active_for_room(&self, room_id: &str) {
        self.active.lock().await.remove(room_id);
    }
}

fn model_error_code(error: &ModelStreamError) -> &'static str {
    match error {
        ModelStreamError::NotHosted => "modelHostUnavailable",
        ModelStreamError::Invalid(_) => "modelRequestRejected",
        ModelStreamError::Transport(_) => "modelTransport",
        ModelStreamError::Rejected { .. } => "modelRejected",
        ModelStreamError::Protocol(_) => "modelProtocol",
    }
}
