# Project Handoff — samta-dashboard

**Date**: 2026-07-07
**Current Commit**: (latest on main, including dual-mode harness)
**Repo**: https://github.com/samwarner16/samta-dashboard
**Goal**: Rust/Axum/Postgres event-sourced autonomous agent dashboard. Workers use pluggable harness for execution. Long-term: control plane for agentic armies on GPU pods (RunPod/Vast.ai) running batch Qwen 2.5 32B inference.

## Key Recent Changes
- **Dual-mode Agent Harness** (core for pod testing):
  - `HARNESS_CONTROLLER=simulated` (default, for local dev/smoke)
  - `HARNESS_CONTROLLER=openrouter` (or api_endpoint) + OPENROUTER_API_KEY + HARNESS_OPENROUTER_MODEL — cloud LLM via OpenRouter (great for harness LLM API testing without local model pain)
  - `HARNESS_CONTROLLER=local_llm` (or `gpu`) + `HARNESS_LLM_BASE_URL=http://localhost:8000/v1` + `HARNESS_LLM_MODEL=...` — harness drives local LLM server (vLLM etc.) spun up on the pod's GPU.
  - Both modes produce identical `WorkItemStarted/Completed/Progress` events. Worker binary auto-selects via `create_executor_from_env()`.
  - Verified with unit tests (no models executed locally — this machine is weak).
- Added `.github/workflows/ci.yml` (build, fmt, clippy, smoke).
- `PLAN.md` with parallel implementation plan for remaining goals (batches, profiles, UI controls, fleet scripts, tests).
- Fixed workspace builds (Tauri stub, etc.).
- README cleaned of stale cargo-make.
- New `Dockerfile` for containerized deploys.
- Worker/harness now ready for GPU pod deployment.

## Current State
- Local boot/smoke works: `BOOT_CLEAN_SLATE=1 ./scripts/boot-all.sh` then `./work/smoke-test.sh`
- Dashboard: http://127.0.0.1:4173/?api=http://127.0.0.1:8080
- Primary binaries: `api`, `worker` (application crate)
- Env for harness on pods:
  ```
  HARNESS_CONTROLLER=local_llm
  HARNESS_LLM_BASE_URL=http://localhost:8000/v1
  HARNESS_LLM_MODEL=Qwen/Qwen2.5-32B-Instruct  # or smaller for test
  WORKER_POLL_MS=1500
  # etc.
  ```
- DB: Postgres + Redis required for worker (event store + projections).
- No real LLM calls in current "local" setup — simulated or stubbed.

## Setup on RunPod (One Pod for Testing the Script)
This is for testing the worker/harness script on a real GPU pod with vLLM.

### 1. Create the Pod on RunPod.io
- Go to https://www.runpod.io/ (console)
- Deploy a new Pod:
  - **Template**: Use "RunPod PyTorch" or "vLLM" template (has CUDA + Python preinstalled). Or "Official" Ubuntu + CUDA.
  - **GPU**: Choose one with sufficient VRAM. For Qwen 2.5 32B testing:
    - Minimum: RTX 4090 (24GB) with 4-bit quant (~18-20GB needed).
    - Better for comfort: A100 40GB/80GB, H100, or 2x 4090.
    - Use "Secure Cloud" or "Community Cloud" for cheaper options. Interruptible for testing.
  - **Container Disk**: 50GB+
  - **Volume Disk**: 100GB+ (for model weights)
  - **Ports to Expose** (important for testing):
    - 22 (SSH)
    - 8000 (vLLM OpenAI compat)
    - 8080 (API)
    - 4173 (Dashboard UI)
    - 5432, 6379 if needed (but use internal)
  - **Env Vars** (optional at create): Set `HARNESS_CONTROLLER=local_llm` etc later.
  - **Start**: Deploy. Note the Pod ID, Public IP, SSH port, and your SSH key (add in settings if not).
- Wait for "Running". Use the "Connect" > "SSH" or web terminal for initial setup.

### 2. SSH into the Pod
```bash
ssh -p <SSH_PORT> root@<PUBLIC_IP> -i ~/.ssh/your_runpod_key
# or use RunPod's one-click web SSH
```

### 3. Bootstrap the Environment (run these commands on pod)
```bash
# Update system
apt-get update && apt-get install -y \
  git curl build-essential python3-pip python3-venv \
  ca-certificates libssl-dev pkg-config

# Install Rust (for building the worker)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
rustup update

# Install vLLM (for local LLM on GPU)
pip install --upgrade pip
pip install vllm  # or torch + vllm for specific CUDA

# Clone the project (latest from GitHub)
git clone https://github.com/samwarner16/samta-dashboard.git
cd samta-dashboard

# (Optional but recommended) Build release binaries
cargo build --release -p api -p application

# Copy example env
cp .env.example .env
# Edit .env with your values (or export later)
```

### 4. Start the LLM Server (vLLM on the pod's GPU)
Use a smaller model first for testing the harness script (e.g. 7B or 1.5B). Scale to 32B when ready.

```bash
# Example: Start vLLM in background (adjust model, gpu-memory-utilization)
nohup python -m vllm.entrypoints.openai.api_server \
  --model Qwen/Qwen2.5-7B-Instruct \
  --port 8000 \
  --gpu-memory-utilization 0.85 \
  --max-model-len 32768 \
  > /tmp/vllm.log 2>&1 &

# Check it's up
sleep 30
curl http://localhost:8000/v1/models
# Should list the model. Tail log if issues: tail -f /tmp/vllm.log
```

For full 32B Qwen2.5, choose pod with enough VRAM and use quantized version or tensor parallel if multi-GPU.

### 5. Start Supporting Services + Dashboard (using docker-compose)
The pod supports Docker.

```bash
# Start postgres, redis, api (builds if no image)
docker compose up -d postgres redis api

# Wait for API healthy
curl http://localhost:8080/health

# Optionally start the static dashboard
# (or use the built one)
nohup python3 -m http.server 4173 --directory crates/dashboard/frontend > /tmp/dashboard.log 2>&1 &
```

Note: The docker-compose `build: .` will now work thanks to the added Dockerfile. It may take time on first build.

### 6. Test the Worker Script / Harness in local_llm mode
This is the key test for the script on the pod.

```bash
# Export harness + DB settings (adjust as needed)
export HARNESS_CONTROLLER=local_llm
export HARNESS_LLM_BASE_URL=http://localhost:8000/v1
export HARNESS_LLM_MODEL=Qwen/Qwen2.5-7B-Instruct
export DATABASE_URL=postgres://user:pass@localhost:5432/agents_db
export REDIS_URL=redis://localhost:6379/
export WORKER_POLL_MS=2000
export WORKER_ITEMS_PER_LOOP=1
# For full test, you may want to create a workspace/run via API first
# curl -X POST http://localhost:8080/api/workspaces -d '{"name":"pod-test"}' ...

# Run the worker (the "script")
cargo run -p application --bin worker
# Or if built: ./target/release/worker

# In another terminal/session, use the harness script:
# ./scripts/run-agent-harness.sh start 1
```

Watch logs. The worker should:
- Poll for assigned work items (from event store)
- Call the local vLLM for "execution" (in harness mode)
- Emit Started / Progress / Completed events

Test the API endpoint mode too by changing `HARNESS_CONTROLLER=api_endpoint` and pointing to an external controller if desired.

Access dashboard at the pod's public URL on port 4173 (if exposed) or via SSH tunnel from your local:
`ssh -L 4173:localhost:4173 -p <port> root@<ip>`

### 7. Useful Pod Commands
- Monitor vLLM: `tail -f /tmp/vllm.log`
- Check GPU: `nvidia-smi`
- Rebuild after code change (git pull): `cargo build --release -p application`
- Full shutdown: `docker compose down`
- Logs for API: `docker logs <api_container_id>`

### Troubleshooting on Pod
- vLLM OOM: Lower `--gpu-memory-utilization` or use smaller/quantized model.
- Rust build slow: Use `--release` only when testing final.
- No GPU in container: Ensure RunPod template has CUDA and `--gpus all` equivalent.
- DB connection: Make sure postgres is up (`docker compose ps`).
- For production-like: Use release binary + systemd or nohup.

## Handoff Notes for "That Computer" (the RunPod)
- The project is fully on GitHub — just `git clone` + `cargo build` on any new machine/pod.
- Use `HARNESS_CONTROLLER=local_llm` for GPU pods.
- For full agent armies later: Use profiles (planned), batch APIs (planned), and external pod orchestration (your vast-ai tools + new launch-pod.sh).
- No local models here (weak machine) — all LLM testing on the pod.
- Next steps from PLAN.md: WS1-WS9 (events, infra, real executors, batches, UI, fleet scripts, tests).
- To resume work: SSH to pod, cd to repo, `cargo check --workspace`, then edit/test the worker/harness.
- Smoke test on pod: Run the local services + a small model + worker.

## Continuation / Resume Prompt (for new terminal on pod or elsewhere)
```
Resume the samta-dashboard project on the RunPod (or target machine).
Read HANDOFF.md and PLAN.md first.
Goal: Test the dual-mode harness (especially local_llm on GPU) using one RunPod. Guide full setup of the pod, run the worker script against local vLLM, verify events flow to dashboard. Then continue parallel work on remaining goals (batches, profiles, etc.).
Current state: Harness supports simulated / api_endpoint / local_llm. Docker support added. Repo on GitHub main.
Commands to start: [paste the bootstrap steps above]
```

Update this HANDOFF.md after major work. Use git for code sync. Destroy pod when done testing to save cost.

Contact / more context: See README.md, PLAN.md, and the GitHub repo.
