use ryu_rooms::{
    model::ParticipantRole,
    store::{CreateRoomInput, RoomStore, RunTerminal, SubmitTurnInput},
};

#[tokio::test]
async fn creates_a_room_and_keeps_invite_secret_out_of_the_snapshot() {
    let store = RoomStore::open_in_memory().expect("in-memory store");
    let created = store
        .create_room(CreateRoomInput {
            model_id: "model-a".to_owned(),
            share_origin: "http://192.168.1.20:7980".to_owned(),
        })
        .await
        .expect("room creation");

    assert!(created.room.id.starts_with("room_"));
    assert!(!created.invite.starts_with("room_"));
    assert!(store
        .snapshot(&created.room.id)
        .await
        .expect("snapshot")
        .is_some());
    assert_eq!(
        store
            .exchange_invite(&created.invite, "Guest")
            .await
            .expect("invite exchange")
            .role,
        ParticipantRole::Guest
    );
    assert!(store
        .raw_invite_for_test(&created.room.id)
        .await
        .expect("raw invite inspection")
        .is_none());
}

#[tokio::test]
async fn idempotent_turns_do_not_create_duplicate_runs() {
    let store = RoomStore::open_in_memory().expect("in-memory store");
    let created = store
        .create_room(CreateRoomInput {
            model_id: "model-a".to_owned(),
            share_origin: "http://192.168.1.20:7980".to_owned(),
        })
        .await
        .expect("room creation");
    let session = store
        .exchange_invite(&created.invite, "Guest")
        .await
        .expect("invite exchange");

    let first = store
        .append_user_turn(
            &session,
            SubmitTurnInput {
                text: "hello".to_owned(),
                idempotency_key: Some("same-turn".to_owned()),
            },
        )
        .await
        .expect("first turn");
    let second = store
        .append_user_turn(
            &session,
            SubmitTurnInput {
                text: "hello".to_owned(),
                idempotency_key: Some("same-turn".to_owned()),
            },
        )
        .await
        .expect("idempotent retry");

    assert_eq!(first.run.run_id, second.run.run_id);
    assert_eq!(
        store
            .snapshot(&created.room.id)
            .await
            .expect("snapshot")
            .expect("room")
            .messages
            .len(),
        1
    );
}

#[tokio::test]
async fn a_second_non_idempotent_turn_is_rejected_while_running() {
    let store = RoomStore::open_in_memory().expect("in-memory store");
    let created = store
        .create_room(CreateRoomInput {
            model_id: "model-a".to_owned(),
            share_origin: "http://192.168.1.20:7980".to_owned(),
        })
        .await
        .expect("room creation");
    let session = store
        .exchange_invite(&created.invite, "Guest")
        .await
        .expect("invite exchange");

    store
        .append_user_turn(
            &session,
            SubmitTurnInput {
                text: "first".to_owned(),
                idempotency_key: None,
            },
        )
        .await
        .expect("first turn");
    let error = store
        .append_user_turn(
            &session,
            ryu_rooms::store::SubmitTurnInput {
                text: "second".to_owned(),
                idempotency_key: None,
            },
        )
        .await
        .expect_err("room must be busy");

    assert_eq!(error.code(), "roomBusy");
}

#[tokio::test]
async fn revoking_an_invite_invalidates_existing_guest_sessions() {
    let store = RoomStore::open_in_memory().expect("in-memory store");
    let created = store
        .create_room(CreateRoomInput {
            model_id: "model-a".to_owned(),
            share_origin: "http://192.168.1.20:7980".to_owned(),
        })
        .await
        .expect("room creation");
    let session = store
        .exchange_invite(&created.invite, "Guest")
        .await
        .expect("invite exchange");

    store
        .revoke_invite(&created.room.id)
        .await
        .expect("revoke invite");

    assert!(store
        .resolve_session(&session.secret)
        .await
        .expect("session lookup")
        .is_none());
    assert!(store
        .exchange_invite(&created.invite, "Again")
        .await
        .is_err());
}

#[tokio::test]
async fn rotating_an_invite_replaces_the_secret_and_invalidates_old_sessions() {
    let store = RoomStore::open_in_memory().expect("in-memory store");
    let created = store
        .create_room(CreateRoomInput {
            model_id: "model-a".to_owned(),
            share_origin: "https://node.example".to_owned(),
        })
        .await
        .expect("room creation");
    let old_session = store
        .exchange_invite(&created.invite, "Old guest")
        .await
        .expect("old invite exchange");

    let issued = store
        .issue_invite(&created.room.id)
        .await
        .expect("invite rotation");
    assert_eq!(issued.share_origin, "https://node.example");
    assert_ne!(issued.invite, created.invite);
    assert!(store
        .resolve_session(&old_session.secret)
        .await
        .expect("old session lookup")
        .is_none());
    assert!(store
        .exchange_invite(&created.invite, "Old invite")
        .await
        .is_err());
    assert!(store
        .exchange_invite(&issued.invite, "New guest")
        .await
        .is_ok());
}

#[tokio::test]
async fn completed_output_is_persisted_once_and_recovery_fails_running_work() {
    let store = RoomStore::open_in_memory().expect("in-memory store");
    let created = store
        .create_room(CreateRoomInput {
            model_id: "model-a".to_owned(),
            share_origin: "https://node.example".to_owned(),
        })
        .await
        .expect("room creation");
    let session = store
        .exchange_invite(&created.invite, "Guest")
        .await
        .expect("invite exchange");
    let accepted = store
        .append_user_turn(
            &session,
            SubmitTurnInput {
                text: "first".to_owned(),
                idempotency_key: Some("first".to_owned()),
            },
        )
        .await
        .expect("turn");

    store
        .append_delta(&created.room.id, &accepted.run.run_id, " ")
        .await
        .expect("delta");
    store
        .append_delta(&created.room.id, &accepted.run.run_id, "answer")
        .await
        .expect("second delta");
    let snapshot = store
        .finish_run(
            &created.room.id,
            &accepted.run.run_id,
            RunTerminal {
                status: ryu_rooms::model::RunStatus::Completed,
                error_code: None,
                error_message: None,
            },
        )
        .await
        .expect("finish");
    assert_eq!(
        snapshot
            .messages
            .iter()
            .filter(|message| message.role == ryu_rooms::model::MessageRole::Assistant)
            .count(),
        1
    );

    let second = store
        .append_user_turn(
            &session,
            SubmitTurnInput {
                text: "second".to_owned(),
                idempotency_key: None,
            },
        )
        .await
        .expect("second turn");
    assert_eq!(store.recover_running().await.expect("recovery"), 1);
    let recovered = store
        .snapshot(&created.room.id)
        .await
        .expect("snapshot")
        .expect("room");
    assert_eq!(recovered.status, ryu_rooms::model::RoomStatus::Failed);
    assert_eq!(
        recovered.current_run.expect("failed run").run_id,
        second.run.run_id
    );
}
