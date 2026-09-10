use tokio::sync::broadcast;

use super::{
    errors::Result,
    store::RuntimeStore,
    types::{EventType, RuntimeEvent},
};

#[derive(Clone)]
pub struct EventEngine {
    store: RuntimeStore,
    sender: broadcast::Sender<RuntimeEvent>,
}

impl EventEngine {
    pub fn new(store: RuntimeStore) -> Self {
        let (sender, _) = broadcast::channel(256);
        Self { store, sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.sender.subscribe()
    }

    pub fn publish(
        &self,
        event_type: EventType,
        source: impl Into<String>,
        target: Option<String>,
        task_id: Option<String>,
        payload: serde_json::Value,
    ) -> Result<RuntimeEvent> {
        let event = RuntimeEvent {
            id: format!("EVT-{}", uuid::Uuid::new_v4()),
            event_type,
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            source: source.into(),
            target,
            task_id,
            payload,
        };
        if self.store.append_event(&event)? {
            let _ = self.sender.send(event.clone());
        }
        Ok(event)
    }
}
