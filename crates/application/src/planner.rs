use anyhow::Result;
use domain::entities::WorkItem;
use uuid::Uuid;

pub struct Planner {
    default_target_item_count: usize,
    max_target_items: usize,
    max_agents: usize,
}

impl Planner {
    pub fn new() -> Self {
        Self {
            default_target_item_count: 4,
            max_target_items: 12,
            max_agents: 16,
        }
    }

    pub fn resolve_target_item_count(&self, requested_item_count: Option<u16>) -> usize {
        requested_item_count
            .unwrap_or(self.default_target_item_count as u16)
            .max(1)
            .min(self.max_target_items as u16) as usize
    }

    pub fn resolve_agent_count(&self, requested_agent_count: Option<u16>) -> usize {
        requested_agent_count.unwrap_or(1).clamp(1, self.max_agents as u16) as usize
    }

    pub async fn decompose_objective(&self, run_id: Uuid, objective: String) -> Result<Vec<WorkItem>> {
        self.decompose_objective_with_target(run_id, objective, self.default_target_item_count)
            .await
    }

    pub async fn decompose_objective_with_target(
        &self,
        run_id: Uuid,
        objective: String,
        requested_item_count: usize,
    ) -> Result<Vec<WorkItem>> {
        let item_target_count = requested_item_count.max(1).min(self.max_target_items);
        let objective_words = objective
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();

        let chunk_width = (objective_words.len() + item_target_count.saturating_sub(1)).saturating_div(item_target_count);
        let chunk_width = chunk_width.max(1);

        let chunks = objective_words
            .chunks(chunk_width)
            .take(item_target_count)
            .filter(|chunk| !chunk.is_empty())
            .map(|chunk| chunk.join(" "))
            .collect::<Vec<_>>();

        if !chunks.is_empty() {
            return Ok(chunks
                .into_iter()
                .enumerate()
                .map(|(index, chunk)| WorkItem {
                    id: Uuid::new_v4(),
                    run_id,
                    description: format!("Step {}: {}", index + 1, chunk),
                    assigned_agent_id: None,
                    status: "unassigned".to_string(),
                })
                .take(item_target_count)
                .collect());
        }

        let fallback_count = self.default_target_item_count.min(item_target_count).min(self.max_target_items);
        let fallback = [
            "Understand objective",
            "Execute supporting checks",
            "Design follow-up actions",
            "Evaluate risks",
            "Summarize outcome",
            "Post results",
        ];

        Ok(fallback
            .iter()
            .take(fallback_count)
            .map(|description| WorkItem {
                id: Uuid::new_v4(),
                run_id,
                description: (*description).to_string(),
                assigned_agent_id: None,
                status: "unassigned".to_string(),
            })
            .collect())
    }

    pub async fn replan_on_failure(&self, failed_item: WorkItem) -> Result<Vec<WorkItem>> {
        Ok(vec![WorkItem {
            id: Uuid::new_v4(),
            run_id: failed_item.run_id,
            description: format!("Retry: {}", failed_item.description),
            assigned_agent_id: None,
            status: "unassigned".to_string(),
        }])
    }
}
