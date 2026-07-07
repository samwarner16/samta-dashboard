use events::{EventEnvelope, EventPayload};

pub struct CostTracker;

impl CostTracker {
    pub fn new() -> Self {
        Self
    }

    pub fn calculate_run_cost(&self, events: &[EventEnvelope]) -> f64 {
        // TODO: Sum up CostMetric from ProgressChunkEmitted events
        events
            .iter()
            .filter_map(|e| match &e.payload {
                EventPayload::ProgressChunkEmitted { cost, .. } => Some(*cost),
                _ => None,
            })
            .sum()
    }
}
