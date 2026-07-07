use anyhow::Result;
use domain::entities::{AgentInstance, WorkItem};

#[derive(Clone)]
pub struct Dispatcher {
    // TODO: Inject Agent Registry
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn assign_items(
        &self,
        mut items: Vec<WorkItem>,
        agents: Vec<AgentInstance>,
    ) -> Result<Vec<WorkItem>> {
        if items.is_empty() {
            return Ok(items);
        }

        let fallback_agent_id = uuid::Uuid::new_v4();
        let assigned = items
            .drain(..)
            .enumerate()
            .map(|(index, mut item)| {
                let assigned_agent_id = if agents.is_empty() {
                    fallback_agent_id
                } else {
                    agents[index % agents.len()].id
                };

                item.status = "assigned".to_string();
                item.assigned_agent_id = Some(assigned_agent_id);
                item
            })
            .collect();

        Ok(assigned)
    }
}
