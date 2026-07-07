use chrono::Utc;
use domain::entities::AgentRun;
use domain::state::AgentRunState;
use events::{EventEnvelope, EventPayload};
use uuid::Uuid;

#[test]
fn test_run_starts_successfully() {
    let run_id = Uuid::new_v4();
    let mut state = AgentRunState::new(AgentRun {
        id: run_id,
        workspace_id: Uuid::new_v4(),
        objective: "Do something".to_string(),
        status: "pending".to_string(),
        created_at: Utc::now(),
    });

    let workspace_id = Uuid::new_v4();
    let event = EventEnvelope {
        id: Uuid::new_v4(),
        occurred_at: Utc::now(),
        actor_id: Uuid::new_v4(),
        resource_id: run_id,
        correlation_id: None,
        causation_id: None,
        revision: 1,
        metadata: serde_json::Value::Null,
        payload: EventPayload::RunStarted {
            workspace_id,
            objective: "Do something".to_string(),
        },
    };

    state.apply(&event).unwrap();
    assert_eq!(state.run.status, "running");
    assert_eq!(state.version, 1);
}
