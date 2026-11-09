use std::{collections::HashMap, sync::Arc};

use tokio::sync::{broadcast, Mutex};

use crate::model::RoomEvent;

const ROOM_EVENT_CAPACITY: usize = 128;

/// Bounded in-process fan-out for guest browser streams. Durable room state stays
/// in SQLite; this hub only carries named changes between snapshot reads.
#[derive(Clone, Default)]
pub struct RoomEventHub {
    rooms: Arc<Mutex<HashMap<String, broadcast::Sender<RoomEvent>>>>,
}

impl RoomEventHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn subscribe(&self, room_id: &str) -> broadcast::Receiver<RoomEvent> {
        let mut rooms = self.rooms.lock().await;
        rooms
            .entry(room_id.to_owned())
            .or_insert_with(|| broadcast::channel(ROOM_EVENT_CAPACITY).0)
            .subscribe()
    }

    pub async fn publish(&self, room_id: &str, event: RoomEvent) -> usize {
        let sender = {
            let mut rooms = self.rooms.lock().await;
            rooms
                .entry(room_id.to_owned())
                .or_insert_with(|| broadcast::channel(ROOM_EVENT_CAPACITY).0)
                .clone()
        };
        sender.send(event).unwrap_or(0)
    }
}
