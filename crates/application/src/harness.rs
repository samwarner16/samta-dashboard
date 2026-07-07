use anyhow::Result;
use async_trait::async_trait;
use events::EventEnvelope;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

use crate::worker::Worker;

/// Context passed to an executor for a single work item.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub run_id: Uuid,
    pub item_id: Uuid,
    pub agent_id: Uuid,
    pub description: String,
}

/// Outcome of attempting to execute a work item.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub success: bool,
    pub effort: i32,
    pub cost: f64,
    pub summary: Option<String>,
    pub error: Option<String>,
}

/// Abstraction for the "agent" that actually performs work on a planned item.
///
/// The harness (executor) can be **controlled in two primary ways** (selectable per worker/profile):
///
/// 1. **API endpoint controlled** (`HARNESS_CONTROLLER=api_endpoint`):
///    The worker/harness posts the `ExecutionContext` (and run context) to a configured
///    external API endpoint. That endpoint returns the outcome (success, effort, summary, etc.).
///    This allows a central "brain" / controller service (which may itself be LLM-powered)
///    to drive agent behavior.
///
/// 2. **LLM on graphics card** (`HARNESS_CONTROLLER=local_llm` or `gpu`):
///    The harness drives a local LLM server spun up on the pod's high-VRAM GPU(s)
///    (e.g. vLLM listening on http://localhost:8000/v1 serving Qwen2.5-32B).
///    Prompts are sent directly to the local endpoint for full local inference.
///
/// Both modes produce the same `ExecutionResult` and event stream so the rest of the
/// system (projections, dashboard, batch controls) is unaware of the backend.
#[async_trait]
pub trait AgentExecutor: Send + Sync {
    async fn execute(&self, ctx: ExecutionContext) -> Result<ExecutionResult>;
}

/// Simulated executor. Matches the previous inline behavior in the worker binary
/// (fixed effort/cost with optional tiny delay for "progress").
pub struct SimulatedExecutor {
    pub base_effort: i32,
    pub base_cost: f64,
    /// If > 0, sleep a fraction of a second to simulate thinking time.
    pub think_ms: u64,
}

impl SimulatedExecutor {
    pub fn new(_agent_id: Uuid, base_effort: i32, base_cost: f64, think_ms: u64) -> Self {
        Self {
            base_effort,
            base_cost,
            think_ms,
        }
    }

    pub fn from_env(agent_id: Uuid) -> Self {
        let effort = std::env::var("WORKER_EFFORT_PER_ITEM")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&e| e > 0)
            .unwrap_or(5);
        let cost = std::env::var("WORKER_COST_PER_ITEM")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&c| c >= 0.0)
            .unwrap_or(0.25);
        let think = std::env::var("WORKER_SIM_THINK_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);
        Self::new(agent_id, effort, cost, think)
    }
}

/// Configuration for which "brain" controls the harness execution.
#[derive(Clone, Debug)]
pub enum HarnessController {
    /// Purely simulated (for testing / smoke).
    Simulated,
    /// Harness delegates to an external API endpoint for decisions/results.
    /// The endpoint receives the context and returns structured outcome.
    ApiEndpoint { endpoint: String },
    /// Harness drives a local LLM server on the graphics card (e.g. vLLM on GPU pod).
    /// base_url typically points to an OpenAI-compatible server like http://localhost:8000/v1 .
    LocalLlm { base_url: String, model: String },
}

impl Default for HarnessController {
    fn default() -> Self {
        HarnessController::Simulated
    }
}

/// Factory to create the appropriate executor based on environment or explicit config.
/// This is the main entry point for choosing "API endpoint" vs "LLM on GPU".
pub fn create_executor_from_env(agent_id: Uuid) -> Box<dyn AgentExecutor> {
    let controller = std::env::var("HARNESS_CONTROLLER")
        .unwrap_or_else(|_| "simulated".into())
        .to_lowercase();

    match controller.as_str() {
        "api" | "api_endpoint" | "remote" => {
            let endpoint = std::env::var("HARNESS_API_ENDPOINT")
                .unwrap_or_else(|_| "http://127.0.0.1:9999/agent-control".to_string());
            Box::new(ApiEndpointExecutor { endpoint })
        }
        "llm" | "local_llm" | "gpu" | "local" => {
            let base_url = std::env::var("HARNESS_LLM_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8000/v1".to_string());
            let model = std::env::var("HARNESS_LLM_MODEL")
                .unwrap_or_else(|_| "Qwen/Qwen2.5-32B-Instruct".to_string());
            Box::new(LocalLlmExecutor { base_url, model })
        }
        _ => Box::new(SimulatedExecutor::from_env(agent_id)),
    }
}

/// Stub implementation: harness is controlled by calling an external API endpoint.
/// In a real deployment this would POST the full context (run objective, history, item desc)
/// to the endpoint and parse a JSON response for the ExecutionResult.
/// Useful when you want a central controller (possibly another LLM service) to drive agents.
pub struct ApiEndpointExecutor {
    pub endpoint: String,
}

#[async_trait]
impl AgentExecutor for ApiEndpointExecutor {
    async fn execute(&self, ctx: ExecutionContext) -> Result<ExecutionResult> {
        // Stub: In production use reqwest::Client to POST to self.endpoint
        // with JSON { run_id, item_id, description, ... } and get back result.
        // For now we simulate a "remote API decision" while proving the wiring.
        if !self.endpoint.is_empty() {
            // Simulate network "call" latency
            sleep(Duration::from_millis(50)).await;
        }

        Ok(ExecutionResult {
            success: true,
            effort: 7,
            cost: 0.35,
            summary: Some(format!(
                "API-controlled result for '{}' via {}",
                ctx.description, self.endpoint
            )),
            error: None,
        })
    }
}

/// Stub implementation for when an LLM is spun up locally on the graphics card(s).
/// The worker pod runs e.g. vLLM serving Qwen2.5-32B (high VRAM) and the harness
/// sends chat/completions requests to the local base_url.
/// This is the mode for decentralized "agentic army" pods.
pub struct LocalLlmExecutor {
    pub base_url: String,
    pub model: String,
}

#[async_trait]
impl AgentExecutor for LocalLlmExecutor {
    async fn execute(&self, ctx: ExecutionContext) -> Result<ExecutionResult> {
        // Stub: Real version would do:
        //   POST {base_url}/chat/completions
        //   with model + messages built from ctx + run history
        //   parse choices[0].message + usage for tokens -> effort/cost
        sleep(Duration::from_millis(80)).await; // simulate GPU inference time

        Ok(ExecutionResult {
            success: true,
            effort: 12,
            cost: 0.0, // local GPU inference usually has different cost model (power + time)
            summary: Some(format!(
                "Local GPU LLM ({}) result for '{}' at {}",
                self.model, ctx.description, self.base_url
            )),
            error: None,
        })
    }
}

#[async_trait]
impl AgentExecutor for SimulatedExecutor {
    async fn execute(&self, ctx: ExecutionContext) -> Result<ExecutionResult> {
        if self.think_ms > 0 {
            sleep(Duration::from_millis(self.think_ms)).await;
        }

        Ok(ExecutionResult {
            success: true,
            effort: self.base_effort,
            cost: self.base_cost,
            summary: Some(format!("simulated completion of: {}", ctx.description)),
            error: None,
        })
    }
}

/// Helper that produces the canonical Started / Progress / Completed (or Failed)
/// events for a work item given an execution result. This centralizes the
/// previous ad-hoc event emission that lived in bin/worker.rs.
pub fn events_for_execution(
    worker: &Worker,
    run_id: Uuid,
    item_id: Uuid,
    mut revision: i32,
    agent_id: Uuid,
    result: &ExecutionResult,
) -> Vec<EventEnvelope> {
    let mut out = vec![];

    out.push(worker.item_started_event(run_id, item_id, revision, agent_id));
    revision += 1;

    if result.success {
        out.push(worker.item_completed_event(
            run_id,
            item_id,
            revision,
            result.effort,
            result.cost,
            agent_id,
        ));
        revision += 1;

        // Emit a progress chunk for parity with historical data (midpoint-ish).
        let p_eff = (result.effort / 2).max(1);
        let p_cost = result.cost / 2.0;
        out.push(worker.progress_chunk_event(run_id, item_id, revision, p_eff, p_cost, agent_id));
    } else {
        let reason = result
            .error
            .clone()
            .unwrap_or_else(|| "executor failed".to_string());
        out.push(worker.item_failed_event(run_id, item_id, revision, reason));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use events::EventPayload;

    #[tokio::test]
    async fn simulated_executor_produces_positive_effort_and_cost() {
        let agent = Uuid::new_v4();
        let exec = SimulatedExecutor::new(agent, 8, 1.5, 0);
        let ctx = ExecutionContext {
            run_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            agent_id: agent,
            description: "test step".into(),
        };
        let res = exec.execute(ctx).await.unwrap();
        assert!(res.success);
        assert_eq!(res.effort, 8);
        assert!((res.cost - 1.5).abs() < 0.0001);
    }

    #[test]
    fn events_for_execution_emits_started_completed_and_chunk() {
        let agent = Uuid::new_v4();
        let w = Worker::new(agent);
        let run = Uuid::new_v4();
        let item = Uuid::new_v4();
        let res = ExecutionResult {
            success: true,
            effort: 6,
            cost: 0.5,
            summary: None,
            error: None,
        };
        let evs = events_for_execution(&w, run, item, 10, agent, &res);
        assert_eq!(evs.len(), 3);
        assert!(matches!(
            evs[0].payload,
            EventPayload::WorkItemStarted { .. }
        ));
        assert!(matches!(
            evs[1].payload,
            EventPayload::WorkItemCompleted { .. }
        ));
        assert!(matches!(
            evs[2].payload,
            EventPayload::ProgressChunkEmitted { .. }
        ));
        assert_eq!(evs[0].revision, 10);
    }

    #[tokio::test]
    async fn factory_selects_simulated_by_default() {
        // Clear any controller env for the test
        std::env::remove_var("HARNESS_CONTROLLER");
        let exec = create_executor_from_env(Uuid::new_v4());
        let ctx = ExecutionContext {
            run_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            description: "factory test".into(),
        };
        let res = exec.execute(ctx).await.unwrap();
        assert!(res.success);
        // Simulated default produces positive synthetic effort
        assert!(res.effort > 0);
    }

    #[tokio::test]
    async fn api_endpoint_mode_produces_controlled_result() {
        std::env::set_var("HARNESS_CONTROLLER", "api_endpoint");
        std::env::set_var("HARNESS_API_ENDPOINT", "http://controller.test/api");
        let exec = create_executor_from_env(Uuid::new_v4());
        let ctx = ExecutionContext {
            run_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            description: "api driven".into(),
        };
        let res = exec.execute(ctx).await.unwrap();
        std::env::remove_var("HARNESS_CONTROLLER");
        std::env::remove_var("HARNESS_API_ENDPOINT");
        assert!(res.success);
        assert!(res.summary.as_deref().unwrap_or("").contains("API-controlled"));
    }

    #[tokio::test]
    async fn local_llm_gpu_mode_produces_local_result() {
        std::env::set_var("HARNESS_CONTROLLER", "local_llm");
        std::env::set_var("HARNESS_LLM_BASE_URL", "http://localhost:8000/v1");
        std::env::set_var("HARNESS_LLM_MODEL", "Qwen/Qwen2.5-32B");
        let exec = create_executor_from_env(Uuid::new_v4());
        let ctx = ExecutionContext {
            run_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            description: "gpu local".into(),
        };
        let res = exec.execute(ctx).await.unwrap();
        std::env::remove_var("HARNESS_CONTROLLER");
        std::env::remove_var("HARNESS_LLM_BASE_URL");
        std::env::remove_var("HARNESS_LLM_MODEL");
        assert!(res.success);
        assert!(res.summary.as_deref().unwrap_or("").contains("Local GPU LLM"));
        // Local mode typically reports 0 monetary cost (power-based accounting elsewhere)
        assert!(res.cost == 0.0);
    }
}
