# RunPod One-Pod Setup Guide for samta-dashboard Testing

Use this to spin up **one** RunPod for testing the worker + harness (especially `local_llm` mode against vLLM on the pod's GPU).

See `HANDOFF.md` for the full handoff document and continuation prompt.

## Prerequisites
- RunPod account + billing set up.
- SSH key added in RunPod settings.
- Local machine (your weak one) only for dashboard control if needed — all heavy/LLM work on pod.

## Step 1: Deploy the Pod
1. In RunPod console → Deploy Pod.
2. Select a **GPU pod** with good VRAM:
   - For initial harness test: RTX 4090 (24GB) or A4000/A5000.
   - For closer to 32B Qwen: A100 40GB+, H100, or multi-GPU.
3. Template: "RunPod PyTorch" (has CUDA) **or** "vLLM" official template.
4. Container/Volume disk: 50GB+ / 100GB+.
5. **Expose these ports**:
   - 22 (SSH)
   - 8000 (vLLM)
   - 8080 (API)
   - 4173 (Dashboard)
6. Deploy. Copy the SSH command and Pod details.

## Step 2: Connect & Bootstrap
SSH in (use RunPod web terminal for first time if easier).

Run the provided onstart/bootstrap script:

```bash
# Option A: If you have the script in the repo (after clone)
chmod +x scripts/runpod-onstart.sh
./scripts/runpod-onstart.sh
```

```bash
# Option B: One-liner bootstrap (paste this if cloning not yet done)
curl -s https://raw.githubusercontent.com/samwarner16/samta-dashboard/main/scripts/runpod-onstart.sh | bash
```

This will:
- Install Rust + vLLM
- Clone the repo
- Build the worker
- Start a test vLLM (7B model — change inside for larger)
- Start docker services if possible

## Step 3: Test the Script (Harness in local_llm mode)
```bash
cd samta-dashboard

# Set harness to use the local GPU LLM
export HARNESS_CONTROLLER=local_llm
export HARNESS_LLM_BASE_URL=http://localhost:8000/v1
export HARNESS_LLM_MODEL=Qwen/Qwen2.5-7B-Instruct   # swap for your 32B quant when ready
export DATABASE_URL=postgres://user:pass@localhost:5432/agents_db
export REDIS_URL=redis://localhost:6379/

# Run the worker directly (the main script under test)
./target/release/worker
# or for debug: cargo run -p application --bin worker

# Alternative: use the harness launcher script
./scripts/run-agent-harness.sh start 2
```

To exercise the full flow:
- In another shell: use curl or the dashboard UI (forward port 4173) to create workspace + run.
- The worker should pick up items and "execute" via the local vLLM (you'll see logs from vLLM + events).

Test the other mode too:
```bash
export HARNESS_CONTROLLER=api_endpoint
export HARNESS_API_ENDPOINT=https://your-controller.example.com/execute
./target/release/worker
```

## Step 4: Access UI / API from Outside
- Use RunPod's "Public IP" + the exposed port, e.g. `http://<pod-ip>:4173/?api=http://<pod-ip>:8080`
- Or from your local machine: `ssh -L 4173:localhost:4173 -p <ssh-port> root@<pod-ip>`

## Step 5: Cleanup
When done testing:
- Stop processes: `pkill -f worker; pkill -f vllm`
- `docker compose down`
- **Delete the pod** in RunPod console to stop billing.

## Tips for Real 32B Qwen
- Pick a pod with 40GB+ VRAM (A100 40/80GB ideal).
- Use a quantized model: `Qwen/Qwen2.5-32B-Instruct-AWQ` or GPTQ.
- Set `--gpu-memory-utilization 0.95` and `--max-model-len` appropriately in vLLM.
- For very large: use multi-GPU pod + tensor-parallel in vLLM.
- Later: wire via agent profiles (see PLAN.md).

## Files Added for This
- `Dockerfile` (makes `docker compose build` work)
- `scripts/runpod-onstart.sh` (the bootstrap you just ran)
- `HANDOFF.md` (full state + resume instructions)
- `docs/runpod-onepod-guide.md` (this file)

See `PLAN.md` → WS7 for future fleet/RunPod launch automation.

Good luck — the harness is now pod-ready! Report back any issues with the script on the actual GPU.