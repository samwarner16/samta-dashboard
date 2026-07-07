use events::EventEnvelope;
use tokio::sync::broadcast;

pub type WsSender = broadcast::Sender<EventEnvelope>;

pub struct WsGateway {
    sender: WsSender,
}

impl WsGateway {
    pub fn new(buffer_size: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer_size);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.sender.subscribe()
    }

    pub fn broadcast(&self, event: EventEnvelope) {
        // Ignore send error if no receivers
        let _ = self.sender.send(event);
    }
}
