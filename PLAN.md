# SAMTA Dashboard — Parallel Implementation Plan
## For Recommended Goals + Long-Term Agentic GPU Armies

**Context**: Rust/Axum/Postgres event-sourced autonomous agent dashboard.  
Current state (as of 2026-07-07): Local boot + smoke works. Supports workspaces, runs, work-item decomposition (naive), run controls, projections, cost/effort, simulated workers via harness script.  
Primary commands: `BOOT_CLEAN_SLATE=1 ./scripts/boot-all.sh`, `./work/smoke-test.sh`, `./scripts/shutdown-all.sh`.  
Dashboard: static HTML at 4173 proxying API at 8080.

**User-provided goals** (prioritized for parallelism):
1. Replace simulated worker execution with a **real agent harness abstraction**.
2. Add **first-class batch-run APIs**.
3. Add **dashboard controls** for launching batches and worker pools.
4. Add **persistent agent profiles and provider/model routing**.
5. Add **stronger projection consistency tests and API integration tests**.
6. Clean up README and remove stale cargo-make assumptions. *(largely complete)*
7. Add **CI** for cargo build/fmt/clippy + smoke (where Docker). *(largely complete)*

**Long-term goal**: Turn this into the **control plane for "agentic armies"** running on disposable high-VRAM GPU pods (RunPod / Vast.ai / "punpods" etc.).
- High-VRAM cards (A100 40/80GB, H100, multiple 4090-class, etc.) running **Qwen2.5-32B** (or similar) via **vLLM / TGI / OpenAI-compatible server**.
- Batch massive numbers of objectives.
- Workers on pods perform real multi-step LLM agent loops (prompting, tool use, reasoning) against the local model endpoint.
- Central event-sourced dashboard for launch, monitoring, pause/resume/cancel/retry, cost (tokens), projections, and fleet oversight.
- Leverage existing user skills in vast-ai, R2, flash fleets, etc. for pod orchestration.

**Core dual-control requirement for the harness** (verified in current implementation):
The `AgentExecutor` (the execution "harness" inside workers) **must be selectable** at runtime:
- **Controlled by an API endpoint**: `HARNESS_CONTROLLER=api_endpoint` + `HARNESS_API_ENDPOINT=...`. The harness posts work item context to the endpoint; the remote API returns the outcome. Ideal for central brain/controller services.
- **Controlled by LLM spun up on graphics card**: `HARNESS_CONTROLLER=local_llm` (or `gpu`) + `HARNESS_LLM_BASE_URL` + `HARNESS_LLM_MODEL`. The harness talks directly to a local vLLM/TGI/etc. server running the Qwen model on the pod's high-VRAM GPU(s). This is the decentralized army mode.
Both modes emit identical events and `ExecutionResult` so the rest of the system (projections, batching, UI, controls) is mode-agnostic. Selection is also possible per-profile in the future. Simulated mode remains for dev/CI.

---

## Parallelization Strategy (Maximize Concurrency)

**Core principles**:
- **Interface first**: Land new `EventPayload` variants, DTOs, traits, and projection shapes early. Independent streams implement against stable contracts.
- **Worktree + branch model** (highly parallel):
  ```bash
  git worktree add -b feat/profiles ../samta-ws-profiles
  git worktree add -b feat/batches ../samta-ws-batches
  git worktree add -b feat/harness-llm ../samta-ws-harness
  git worktree add -b feat/ui-fleet ../samta-ws-ui
  # etc.
  ```
  Each worktree can be developed/tested largely independently (or with light rebases on a `plan/main`).
- **Crate/layer isolation**: `events/` and `domain/` are pure and fast to change. `infra/` next. `application/` and `api/` can proceed in parallel once contracts exist.
- **Feature flags / env fallbacks**: Keep `SimulatedExecutor` as default. Enable real executors via `WORKER_EXECUTOR=llm_compat` + profile.
- **Test & CI early**: Streams can add their own unit tests immediately. Smoke evolves incrementally.
- **6+ streams feasible** because frontend (single-file HTML/JS), scripts, and Rust layers have loose coupling.
- Use `cargo check -p <specific>` + targeted tests for fast feedback per stream.
- For GPU army work: Pod scripts and deployment are almost completely parallel to core Rust changes (they consume the final APIs).

**Critical path is short** if interfaces are agreed in the first 1-2 days of parallel work.

---

## High-Level Waves (Overlap Where Possible)

**Wave 0 (baseline — mostly done)**: Harness skeleton (simulated via trait), CI skeleton + smoke hardening, README cleanup, fmt fixes, build fixes for workspace, basic tests.

**Wave 1 — Foundation (maximum parallelism)**:
- Events + Domain models (profiles + batch concepts)
- Infra (migrations + projections + replay)
- Test expansion (consistency)
- Early harness trait hardening + docs
- Deployment skeleton (Docker + pod scripts)

**Wave 2 — Core Features (parallel after Wave 1 contracts)**:
- Real LLM executor impl(s) + profile injection
- Batch orchestration + APIs
- API routes + DTOs for profiles/batches/workers
- Worker registration + pool management

**Wave 3 — Observability + Control (UI + fleet)**:
- Dashboard UI for profiles, batch launch, worker pools, model attribution
- Full harness script + fleet launchers
- Cost/token tracking improvements

**Wave 4 — Production & Scale**:
- Advanced agent loops inside executors
- Dynamic fleet scaling signals
- Real GPU pod end-to-end (Qwen batch)
- Polish, more tests, docs, examples

---

## Detailed Parallel Workstreams

### WS1: Events, Domain Models & Types (events/ + domain/)
**Goal coverage**: 4 (profiles), 2 (batches), 1 (harness data)
**Parallelism**: Highest — almost everything depends on the shapes but not the impls. Do this first in parallel with infra tests.

**Tasks**:
- Enrich `EventPayload::AgentProfileCreated` (and possibly add `AgentProfileUpdated`):
  ```rust
  AgentProfileCreated {
      name: String,
      role: String,
      provider: String,           // "openai_compat", "vllm", "anthropic", ...
      model: String,              // "Qwen/Qwen2.5-32B-Instruct-AWQ"
      base_url: Option<String>,
      cost_cap: f64,
      params: serde_json::Value,  // temperature, max_tokens, etc.
  }
  ```
- New events:
  - `BatchCreated { name, objective_template?, target_runs, profile_id? }`
  - `BatchRunAdded { batch_id, run_id }`
  - `BatchStatusChanged`
  - `WorkerRegistered { worker_id, profile_ids, capacity, model_info, endpoint }` (for army visibility)
  - Optional: `InferenceUsageEmitted` (for real token costs)
- Update `domain::entities`:
  - `AgentProfile` with full routing fields.
  - New `Batch`, `InferenceEndpoint`, `WorkerRegistration`.
  - Keep `WorkItem` but consider richer description/payload later.
- Update `EventType` enum and `event_type()` matcher.
- Update any replay/state helpers that pattern-match events (they are currently duplicated — opportunity to consolidate).
- Add serde derives / JSON examples.

**Deliverables**: Small PR with types + unit tests. Unblocks all other WS.

**Long-term tie-in**: Profiles become the "unit" of army specialization (different Qwen variants, different tool sets, different pods).

### WS2: Persistence, Migrations & Replay (infra/)
**Goal coverage**: 4, 2, 5 (consistency)
**Parallelism**: High — can start as soon as WS1 defines payloads. Independent of API/UI.

**Tasks**:
- New migration files under `migrations/` (sqlx discovers them):
  - `003_agent_profiles.sql`: `agent_profiles_projection` table (profile_id, workspace, name, provider, model, base_url, cost_cap, ...).
  - `004_batches.sql`: `batches_projection`, `batch_runs` join table.
  - `005_workers.sql` (optional early): worker registrations + last_seen.
- Extend `ProjectionStore` trait with:
  - `upsert_agent_profile_projection(...)`
  - `get_profiles(...)`, batch equivalents, etc.
- Implement in `PostgresProjectionStore`.
- Extend `rebuild_projections` (and the in-worker replay logic) to handle new event types and populate new tables.
- Update `migrations.rs` if needed (usually automatic).
- Add helpers for loading profiles by ID (used by harness).

**Tests** (can be written in parallel):
- Pure reconstruction tests: feed a sequence of events (including profile + batch) → assert projection rows.
- Cross-check that `run_states_from_events` (worker) and `rebuild_projections` agree on core fields.

**Long-term**: Projections become the source of truth for "which models are available in the army right now".

### WS3: Agent Harness Evolution & Real Execution (application/)
**Goal coverage**: 1 (primary), 4 (routing), parts of 2/3
**Parallelism**: Medium-high. The trait is already landed. Can prototype real executor while WS1/2 land data.

**Current (post-Wave 0)**: `AgentExecutor` trait + factory, `SimulatedExecutor`, `ApiEndpointExecutor`, `LocalLlmExecutor`, `events_for_execution` helper. Worker binary selects via `create_executor_from_env`. Dual modes verified (see below).

**Dual harness control (API endpoint vs local GPU LLM) — explicit requirement**:
- `HARNESS_CONTROLLER=simulated` (default, synthetic)
- `HARNESS_CONTROLLER=api_endpoint` + `HARNESS_API_ENDPOINT`: harness is **controlled by the API endpoint**. The executor acts as a client that asks the remote endpoint for the result of the work item.
- `HARNESS_CONTROLLER=local_llm` (aliases: gpu, local): harness is driven by **LLM spun up on the graphics card**. Points at local OpenAI-compat server (vLLM on the pod GPU).
The factory + stubs are implemented and tested. Real network/LLM calls added when `reqwest` is introduced in later waves. Both paths must remain drop-in replacements.

**Tasks**:
- (Done in baseline) Implement `create_executor_from_env` + `ApiEndpointExecutor` + `LocalLlmExecutor` stubs that produce correct `ExecutionResult` and events.
- Add `reqwest` dependency and flesh out real HTTP calls in the two non-sim modes (WS3 proper).
- Make executor construction profile-aware (once profiles exist): read controller type + connection details from `AgentProfile`.
- Update worker binary to use the factory (done).
- Support richer context passing (run objective, prior chunks) for real LLM prompts.
- Optional: multi-turn agent loop inside the LLM-controlled executors.
- Add usage/token extraction for real cost accounting.
- Keep full backward compatibility for simulated + smoke/CI.

**Add dependency**: `reqwest = { version = "0.11", features = ["json", "rustls-tls"] }` (or async-openai for convenience).

**Long-term vision enablement**:
- On a GPU pod: vLLM listens on localhost:8000 → worker binary configured with `base_url=http://localhost:8000/v1`, `model=Qwen...`.
- Same binary + profile works for remote endpoints too.
- Batch Qwen calls: workers can process many items concurrently (tune `WORKER_ITEMS_PER_LOOP` + pod GPU memory).

### WS4: Batch APIs & Orchestration Logic (application/ + api/)
**Goal coverage**: 2 (primary), 3 (support)
**Parallelism**: Can proceed once WS1 defines Batch events. Orchestrator changes are localized.

**Tasks**:
- New DTOs in `api/src/dto.rs`: `CreateBatchRequest`, `BatchResponse`, `BatchSummary`, etc.
- Orchestrator:
  - `create_batch(...)` → emit `BatchCreated`.
  - `start_batch(...)` → create N runs (reuse existing `start_run` logic), link via `BatchRunAdded` events.
  - Batch-level status aggregation (projection-driven or replay).
  - `pause_batch`, `resume_batch`, `cancel_batch` (fan-out to runs or new batch status events).
- Extend `Planner` (or new `BatchPlanner`) for batch-level decomposition if needed.
- Dispatcher updates to respect profile affinity at batch level.
- API routes:
  - `POST /api/batches`
  - `GET /api/batches`, `GET /api/batches/:id`
  - `POST /api/batches/:id/{pause,resume,cancel}`
  - Optional: `POST /api/batches/:id/runs` (ad-hoc add).

**Long-term**: A "batch" is how you launch an army of 50–500 related agent runs against Qwen.

### WS5: API Layer & Wiring (api/)
**Goal coverage**: 2, 3, 4
**Parallelism**: High after DTOs + orchestrator methods exist. Can be developed against mocks initially.

**Tasks**:
- Add profile endpoints (use orchestrator `create_agent_profile` + new list/get via projections or event scan):
  - `POST /api/agent-profiles`
  - `GET /api/agent-profiles`
  - `GET /api/agent-profiles/:id`
- Worker registration endpoint (lightweight):
  - `POST /api/workers/register` (idempotent, updates last_seen + capabilities).
- Wire new orchestrator methods into routes.
- Update WebSocket broadcast for new event types (already fairly generic).
- Add query helpers for "runs in batch", "profiles for workspace".

**Auth note**: Current system is open. Add JWT stub or note for later if fleets are exposed.

### WS6: Dashboard UI Controls (crates/dashboard/frontend/)
**Goal coverage**: 3 (primary), 2/4 (consumption)
**Parallelism**: Extremely high — single file, can be edited in parallel with backend. Use the `?api=` param.

**Current UI**: Metrics, workspace/run creation (with target_item/agent_count), run list/detail, controls (pause etc.), live events, cost history.

**Tasks**:
- Profiles panel:
  - Form: name, role, provider (select or text), model, base_url, cost_cap.
  - Create button → POST.
  - List/table of known profiles (poll `/api/agent-profiles`).
- Batch launcher:
  - "Launch Batch" section: name, number of runs or list of objectives, profile selector (or "auto"), target items per run.
  - Button creates batch + N runs.
- Worker / Pool status:
  - Section showing harness workers or registered workers (from new endpoint).
  - Buttons or instructions: "Start local pool (N)", link to run-agent-harness.
  - For remote: show "desired fleet size" or registered pods.
- Run/Batch detail: show assigned profile + model (when available).
- Minor: model attribution in timeline/history.

Keep everything vanilla JS + fetch + the existing polling/WS pattern. No framework.

### WS7: Scripts, Docker & GPU Pod Fleet (scripts/ + root)
**Goal coverage**: 3 (worker pools), long-term army
**Parallelism**: Almost completely independent of Rust code changes. Can start immediately.

**Tasks**:
- Add `Dockerfile` (multi-stage) for the worker binary (and optionally api).
- Enhance or add `scripts/run-agent-harness.sh`:
  - Support `WORKER_PROFILE_ID`, `MODEL_BASE_URL`, `CENTRAL_API_URL`.
  - Better logging, health.
- New `scripts/launch-pod.sh` or `fleet/` examples:
  - Uses `vastai` CLI or RunPod API (or curl) to create pod with high-VRAM template.
  - On pod boot: install vLLM, download Qwen2.5-32B (quant), start server, start worker binary.
  - Pass env: profile to use, central dashboard URL.
- Update `docker-compose.yml` example for "local army" (postgres + redis + multiple workers + a fake vLLM stub).
- Worker self-registration on startup (call API).
- Harness control improvements: affinity by profile, dynamic scaling signals.

**Long-term**: External "army controller" (your existing vast-ai tooling) watches the dashboard for "desired capacity by profile" and spins/destroys pods accordingly.

### WS8: Tests, Consistency & CI Hardening (everywhere + work/)
**Goal coverage**: 5 (primary), supports all
**Parallelism**: High — unit tests can be written against interfaces from day 1.

**Tasks**:
- Projection consistency tests (in infra or a new `tests/` crate):
  - Build a rich event log (workspace + profiles + batch + runs + items + completions).
  - Run `rebuild_projections`.
  - Assert exact rows in all projection tables + derived metrics.
- Harness contract tests: Simulated + (future) LLM executor produce valid event sequences.
- API integration: Use `#[tokio::test]` + test DB or `testcontainers` (add dep) for route + projection flows.
- Evolve `work/smoke-test.sh`:
  - Create profile → launch batch → assert model attribution appears.
  - Parameterized executor mode.
- CI:
  - Make smoke job more robust (retries, better logging).
  - Optional GPU job (very expensive — manual trigger or separate workflow).
  - Add `cargo test --workspace --all-features`.
- Fix pre-existing clippy items gradually so `-D warnings` can be turned on.

### WS9: Documentation, Examples & Vision (README + new docs/)
**Parallelism**: Ongoing / low conflict.

**Tasks**:
- Update README with new endpoints, profile + batch examples, "local army" docker-compose.
- New files:
  - `docs/AGENT_PROFILES.md`
  - `docs/BATCHES_AND_FLEETS.md`
  - `docs/GPU_POD_DEPLOYMENT.md` (Qwen + vLLM + worker on RunPod/Vast)
  - `docs/EXECUTOR_IMPLEMENTATION.md` (how to write a real one)
- Example payloads and curl commands for batch Qwen usage.
- Link back to long-term vision.

---

## Architecture Notes for GPU Army Vision

1. **Profile = Specialization**:
   - One profile per "role + model combo".
   - A high-VRAM pod registers workers that advertise they can serve certain profiles.

2. **Execution Flow (future)**:
   - Planner decomposes objective → WorkItems (richer descriptions or structured tasks later).
   - Dispatcher assigns using profile + current worker capacity (from registrations).
   - Worker selects executor per `HARNESS_CONTROLLER` (or future profile field):
     - `api_endpoint`: posts context to the configured controller API → receives result.
     - `local_llm` / gpu: talks to **local LLM** (vLLM etc.) spun up on the pod's graphics card(s) at the profile's `base_url`.
   - `executor.execute(ctx)` emits real `WorkItem*` + `ProgressChunk` + usage events.
   - Same binary supports both modes. Multiple workers per pod (careful VRAM sharing / tensor parallelism).

3. **Batching at Scale**:
   - One "batch" launch = hundreds of objectives.
   - Dashboard shows aggregate cost/effort + per-model breakdown.
   - Controls cascade to individual runs/items.

4. **Cost & Observability**:
   - Extend projections with token counts when executors report usage.
   - Per-profile cost caps + alerts (future).

5. **Pod Lifecycle** (outside core for now):
   - Use existing vast-ai tooling + new thin scripts that point workers at the dashboard.
   - Workers are ephemeral; events are the source of truth.

---

## Risks, Mitigations & Open Questions

**Risks**:
- LLM responses are non-deterministic → make executor return both raw output + normalized result.
- Pod cost blow-up → profile caps + kill signals in projections.
- Event volume at army scale → good projections + (later) compaction/snapshotting.
- vLLM + Qwen 32B setup complexity on pods → provide battle-tested startup scripts + recommended quant + template IDs.
- Parallel dev drift → frequent interface reviews + shared `PLAN.md` updates.

**Open Questions** (decide early in Wave 1):
- Exact fields on `AgentProfileCreated` (base_url vs. endpoint id)?
- Does a batch own its own set of profiles, or reuse workspace profiles?
- Worker claim model vs. pure push assignment?
- How much agent "loop" logic lives in the Rust executor vs. inside the LLM prompt ("agentic" scaffolding)?
- Token pricing for local runs (0 or synthetic)?

---

## Verification & Success Criteria

**Must verify dual harness control** (completed baseline verification):
- `cargo test -p application harness` passes with tests for:
  - factory defaults to simulated
  - `HARNESS_CONTROLLER=api_endpoint` + endpoint → produces "API-controlled" results
  - `HARNESS_CONTROLLER=local_llm` + base_url/model → produces "Local GPU LLM" results (cost=0, GPU-style)
- Same worker binary can be launched with either mode via env (no code change).
- Both modes feed identical events into the event store / projections / dashboard.
- Simulated mode untouched for CI/smoke.

Other criteria:
- All original smoke tests still pass (sim path).
- New flows:
  - Create profile (with provider/model + controller) → create batch using that profile → workers (API-controlled or local-GPU-LLM) process items → UI shows attribution.
- Projection consistency test passes for full new event set.
- `cargo fmt -- --check && cargo clippy --workspace && cargo test --workspace` clean.
- Full workspace `cargo check --workspace` succeeds.
- CI smoke job green (exercises at least simulated + one other mode).
- Documented "launch 8 Qwen agents locally (GPU mode)" and "central API controller mode" in docs.
- (Stretch) End-to-end with a real local vLLM Qwen instance on a high-VRAM setup.

---

## Immediate Next Steps (after accepting this plan)

1. Commit / snapshot current state (harness + CI + README work).
2. Create worktrees for WS1 (events/domain) and WS2 (infra) — highest leverage.
3. Agree on final `AgentProfileCreated` shape (small design sync).
4. Land WS1 types → unblock everyone.
5. Start WS7 (pod scripts) and WS6 (UI) in parallel immediately — they have almost no blockers.
6. Evolve smoke + add one consistency test per stream.

This structure allows the team (or agent swarm) to make forward progress on 4–6 tracks simultaneously while keeping the path to "Qwen 32B agent armies on high-VRAM pods" clear and incremental.

---

*Generated as part of session on 2026-07-07. Update this file as decisions are made.*