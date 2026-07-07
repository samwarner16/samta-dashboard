#!/bin/bash
# onstart / bootstrap script for RunPod (or similar GPU pods)
# Use this in RunPod's "Docker Start Command" or copy and run after SSH.
# Purpose: Quickly get the samta-dashboard worker + local LLM (vLLM) ready for testing the harness script.

set -euo pipefail

echo "=== RunPod one-pod bootstrap for samta-dashboard harness test ==="

# 1. System deps (idempotent)
apt-get update -qq
apt-get install -y -qq git curl build-essential python3-pip python3-venv ca-certificates libssl-dev pkg-config

# 2. Rust (for worker)
if ! command -v cargo &> /dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
source "$HOME/.cargo/env"
rustup update stable

# 3. Clone or update repo
if [ ! -d "samta-dashboard" ]; then
  git clone https://github.com/samwarner16/samta-dashboard.git
fi
cd samta-dashboard
git pull --ff-only || true

# 4. Build the worker (release for speed)
cargo build --release -p application --bin worker
echo "Worker binary ready: target/release/worker"

# 5. vLLM + small test model (change model for real 32B test)
pip install --upgrade -q pip
pip install -q vllm

# Start vLLM in background if not running (use a small model for initial test)
if ! curl -s http://localhost:8000/v1/models > /dev/null; then
  echo "Starting vLLM with test model (Qwen2.5-7B)..."
  nohup python -m vllm.entrypoints.openai.api_server \
    --model Qwen/Qwen2.5-7B-Instruct \
    --port 8000 \
    --gpu-memory-utilization 0.8 \
    --max-model-len 16384 \
    > /tmp/vllm.log 2>&1 &
  sleep 20
  echo "vLLM starting... tail /tmp/vllm.log"
fi

# 6. Start supporting stack (postgres/redis/api) using docker compose if available
if command -v docker &> /dev/null && [ -f docker-compose.yml ]; then
  echo "Starting postgres/redis/api via docker compose..."
  docker compose up -d postgres redis api || echo "docker compose may need build time"
  sleep 10
else
  echo "Note: docker compose not used or not available. For full test you may need to start DB/API manually."
fi

# 7. Example: Run the worker in local_llm mode (the key test)
echo ""
echo "=== Ready to test the script ==="
echo "To test harness in local_llm (GPU) mode:"
echo "  export HARNESS_CONTROLLER=local_llm"
echo "  export HARNESS_LLM_BASE_URL=http://localhost:8000/v1"
echo "  export HARNESS_LLM_MODEL=Qwen/Qwen2.5-7B-Instruct"
echo "  export DATABASE_URL=postgres://user:pass@localhost:5432/agents_db"
echo "  export REDIS_URL=redis://localhost:6379/"
echo "  ./target/release/worker"
echo ""
echo "Or use the harness launcher:"
echo "  ./scripts/run-agent-harness.sh start 1"
echo ""
echo "Check dashboard on port 4173 or API on 8080 (if exposed in pod)."
echo "For full 32B: change the vLLM model + pick a pod with enough VRAM (A100/H100 recommended)."
echo "Bootstrap complete. Happy testing!"
