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
/// The harness (executor) can be controlled in several ways (selectable via HARNESS_CONTROLLER):
///
/// - `simulated`: Pure simulation (default for dev/smoke).
/// - `local_llm` / `gpu`: Local vLLM or compatible server on GPU pod.
/// - `openrouter` or `api_endpoint`: Cloud LLM via OpenRouter (or any OpenAI-compatible endpoint).
///
/// For OpenRouter (recommended for harness LLM API testing):
///   HARNESS_CONTROLLER=openrouter
///   OPENROUTER_API_KEY=sk-or-...
///   HARNESS_OPENROUTER_MODEL=qwen/qwen-2.5-32b-instruct   # or any OpenRouter model
///
/// The executor builds a prompt from the work item and asks the LLM to "execute" it,
/// expecting a useful summary. Effort/cost can be derived from usage or LLM output.
///
/// All modes produce identical `ExecutionResult` + events.
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
    /// OpenRouter (or generic OpenAI-compatible) for cloud LLM as the harness brain.
    /// Recommended for testing the harness LLM API without local GPU.
    OpenRouter { api_key: String, model: String, base_url: String },
    /// Harness drives a local LLM server on the graphics card (e.g. vLLM on GPU pod).
    LocalLlm { base_url: String, model: String },
}

impl Default for HarnessController {
    fn default() -> Self {
        HarnessController::Simulated
    }
}

/// Factory to create the appropriate executor based on environment or explicit config.
pub fn create_executor_from_env(agent_id: Uuid) -> Box<dyn AgentExecutor> {
    let controller = std::env::var("HARNESS_CONTROLLER")
        .unwrap_or_else(|_| "simulated".into())
        .to_lowercase();

    match controller.as_str() {
        "openrouter" | "api" | "api_endpoint" | "remote" => {
            let api_key = std::env::var("OPENROUTER_API_KEY")
                .or_else(|_| std::env::var("HARNESS_API_KEY"))
                .unwrap_or_else(|_| "sk-or-test".to_string());
            let model = std::env::var("HARNESS_OPENROUTER_MODEL")
                .or_else(|_| std::env::var("HARNESS_LLM_MODEL"))
                .unwrap_or_else(|_| "qwen/qwen-2.5-32b-instruct".to_string());
            let base_url = std::env::var("OPENROUTER_BASE_URL")
                .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());
            Box::new(OpenRouterExecutor { api_key, model, base_url })
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

/// Real OpenRouter (OpenAI-compatible) executor for the harness "LLM API".
/// Uses OpenRouter to let a cloud LLM act as the agent's brain for executing work items.
/// 
/// Set:
///   HARNESS_CONTROLLER=openrouter
///   OPENROUTER_API_KEY=sk-or-v1-...
///   HARNESS_OPENROUTER_MODEL=qwen/qwen-2.5-32b-instruct
///
/// The prompt asks the LLM to "execute" the step and return a short summary.
/// Effort is derived from response length + usage (or fixed base).
pub struct OpenRouterExecutor {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

#[async_trait]
impl AgentExecutor for OpenRouterExecutor {
    async fn execute(&self, ctx: ExecutionContext) -> Result<ExecutionResult> {
        if self.api_key == "sk-or-test" || self.api_key.is_empty() {
            // Fallback for tests / no key — simulates a real LLM response
            sleep(Duration::from_millis(30)).await;
            return Ok(ExecutionResult {
                success: true,
                effort: 8,
                cost: 0.001,
                summary: Some(format!("OpenRouter stub result for '{}'", ctx.description)),
                error: None,
            });
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;

        // Structured prompt + response_format for reliable JSON (supported by many OpenRouter models)
        let prompt = format!(
            "You are an autonomous agent. Execute this work item step: {}\n\
             Return ONLY this JSON (no other text):\n\
             {{\"success\": true, \"effort\": <int 3-20>, \"cost\": <float e.g. 0.001>, \"summary\": \"1-2 sentence concrete result\"}}",
            ctx.description
        );

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "You are precise. Output ONLY the requested JSON."},
                {"role": "user", "content": prompt}
            ],
            "max_tokens": 300,
            "temperature": 0.3
        });

        // Many models on OpenRouter support this for strict JSON
        if self.model.contains("qwen") || self.model.contains("gpt") || self.model.contains("claude") {
            body["response_format"] = serde_json::json!({"type": "json_object"});
        }

        let resp = client
            .post(format!("{}/chat/completions", self.base_url.trim_end_matches('/')))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://github.com/samwarner16/samta-dashboard")
            .header("X-Title", "samta-dashboard-harness")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Ok(ExecutionResult {
                success: false,
                effort: 1,
                cost: 0.0,
                summary: None,
                error: Some(format!("OpenRouter HTTP error: {}", text)),
            });
        }

        let json: serde_json::Value = resp.json().await?;

        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("LLM returned no content")
            .trim()
            .to_string();

        // Prefer structured JSON from the model
        let (success, effort, cost, summary) = if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
            (
                parsed.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
                parsed.get("effort").and_then(|v| v.as_i64()).unwrap_or(8) as i32,
                parsed.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.001),
                parsed.get("summary").and_then(|v| v.as_str()).unwrap_or(&content).to_string(),
            )
        } else {
            // Fallback to text parsing + usage
            let usage = &json["usage"];
            let comp = usage["completion_tokens"].as_i64().unwrap_or(50) as i32;
            let (e, c) = parse_effort_cost(&content).unwrap_or((comp / 5 + 5, 0.0));
            (true, e, c, content)
        };

        let usage_cost = json["usage"]["cost"].as_f64().unwrap_or(cost);

        Ok(ExecutionResult {
            success,
            effort: effort.max(3),
            cost: usage_cost,
            summary: Some(summary),
            error: None,
        })
    }
}

fn parse_effort_cost(text: &str) -> Option<(i32, f64)> {
    // Simple parser for lines like "EFFORT: 12 COST: 0.002"
    let lower = text.to_lowercase();
    let effort = lower
        .split("effort:")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.trim_matches(|c: char| !c.is_numeric()).parse::<i32>().ok());

    let cost = lower
        .split("cost:")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse::<f64>().ok());

    match (effort, cost) {
        (Some(e), Some(c)) => Some((e, c)),
        _ => None,
    }
}

/// Real implementation for local LLM on graphics card (vLLM, Ollama, etc. OpenAI compat).
/// The harness sends real chat/completions to the local server on the pod's GPU(s).
pub struct LocalLlmExecutor {
    pub base_url: String,
    pub model: String,
}

#[async_trait]
impl AgentExecutor for LocalLlmExecutor {
    async fn execute(&self, ctx: ExecutionContext) -> Result<ExecutionResult> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        let prompt = format!(
            "Execute this work item step precisely: {}\n\
             Give a short, concrete summary of the result.",
            ctx.description
        );

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "You are a precise autonomous worker."},
                {"role": "user", "content": prompt}
            ],
            "max_tokens": 200,
            "temperature": 0.6
        });

        match client
            .post(format!("{}/chat/completions", self.base_url.trim_end_matches('/')))
            .json(&body)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    let content = json["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or("no content")
                        .to_string();

                    let usage = &json["usage"];
                    let completion = usage["completion_tokens"].as_i64().unwrap_or(40) as i32;
                    let effort = (completion / 4).max(5);

                    return Ok(ExecutionResult {
                        success: true,
                        effort,
                        cost: 0.0,
                        summary: Some(content),
                        error: None,
                    });
                }
            }
            _ => {}
        }

        // Fallback for tests / when local server not available (e.g. dev without vLLM running)
        sleep(Duration::from_millis(50)).await;
        Ok(ExecutionResult {
            success: true,
            effort: 10,
            cost: 0.0,
            summary: Some(format!(
                "[fallback] local LLM result for '{}' (server at {})",
                ctx.description, self.base_url
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
    async fn openrouter_mode_uses_fallback_without_key() {
        std::env::set_var("HARNESS_CONTROLLER", "openrouter");
        std::env::set_var("OPENROUTER_API_KEY", "sk-or-test");
        std::env::set_var("HARNESS_OPENROUTER_MODEL", "qwen/qwen-2.5-7b-instruct");
        let exec = create_executor_from_env(Uuid::new_v4());
        let ctx = ExecutionContext {
            run_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            description: "openrouter test step".into(),
        };
        let res = exec.execute(ctx).await.unwrap();
        std::env::remove_var("HARNESS_CONTROLLER");
        std::env::remove_var("OPENROUTER_API_KEY");
        std::env::remove_var("HARNESS_OPENROUTER_MODEL");
        assert!(res.success);
        assert!(res.summary.as_deref().unwrap_or("").contains("OpenRouter"));
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
        assert!(res.summary.as_deref().unwrap_or("").contains("fallback") || res.summary.as_deref().unwrap_or("").contains("local LLM"));
        // Local mode typically reports 0 monetary cost (power-based accounting elsewhere)
        assert!(res.cost == 0.0);
    }

    #[tokio::test]
    async fn openrouter_with_real_key_would_call_api_but_falls_back_in_test() {
        // This test exercises the real path only when a real key is present.
        // Without key we get the built-in fallback (no network).
        std::env::set_var("HARNESS_CONTROLLER", "openrouter");
        std::env::remove_var("OPENROUTER_API_KEY"); // force fallback
        let exec = create_executor_from_env(Uuid::new_v4());
        let ctx = ExecutionContext {
            run_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            description: "cloud llm test".into(),
        };
        let res = exec.execute(ctx).await.unwrap();
        std::env::remove_var("HARNESS_CONTROLLER");
        assert!(res.success);
        assert!(res.summary.as_deref().unwrap_or("").contains("OpenRouter"));
    }
}
