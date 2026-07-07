use crate::entities::AgentRun;
use events::EventEnvelope;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StateError {
    #[error("invalid state transition")]
    InvalidTransition,
}

// Example Aggregate State for AgentRun
#[derive(Debug, Clone)]
pub struct AgentRunState {
    pub run: AgentRun,
    pub version: i32,
}

impl AgentRunState {
    pub fn new(run: AgentRun) -> Self {
        Self { run, version: 0 }
    }

    pub fn apply(&mut self, event: &EventEnvelope) -> Result<(), StateError> {
        // TODO: Implement full state transitions based on event types
        match event.event_type() {
            events::EventType::RunStarted => {
                self.run.status = "running".to_string();
            }
            events::EventType::RunCompleted => {
                self.run.status = "completed".to_string();
            }
            _ => {}
        }
        self.version = event.revision;
        Ok(())
    }
}
