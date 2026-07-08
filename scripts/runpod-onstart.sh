#!/bin/bash
# Parallelized onstart / bootstrap for RunPod (H200 or similar high-VRAM GPU pods)
# Purpose: Get samta-dashboard + harness (local_llm or openrouter) ready FAST to minimize expensive pod time.
#
# Usage in RunPod:
# - Use a template with CUDA (e.g. PyTorch or Ubuntu + CUDA)
# - In "Start Command" or after SSH: bash <(curl -s https://raw.githubusercontent.com/samwarner16/samta-dashboard/main/scripts/runpod-onstart.sh)
# - Or copy the script to the pod and run.
#
# Parallelization strategy to minimize wall time:
# - Run independent long tasks (rustup, git clone, pip, docker pulls, cargo build) in background as soon as possible.
# - apt first (needed for tools).
# - Overlap CPU/network heavy steps.
# - Only serialize when dependencies exist (e.g. clone before cd/build).
# - Start vLLM (GPU) and DBs as soon as their prereqs are done.
# - Use nohup for services.
#
# Supports both local_llm (vLLM on GPU) and openrouter (cloud LLM for harness) modes.
# For fastest setup when testing harness LLM API: set env HARNESS_CONTROLLER=openrouter before running (skips heavy vLLM).
#
# Env overrides (export before running script):
#   SKIP_VLLM=1          # skip vLLM install/start (use for openrouter mode)
#   VLLM_MODEL=...       # default Qwen/Qwen2.5-7B-Instruct
#   USE_VENV=1           # use python venv to avoid system conflicts
#
# After bootstrap: follow the printed "Ready to test" instructions.
# Total time should be dominated by the slowest single task (large pip wheel or cargo) instead of sum.

set -euo pipefail

echo "=== Parallel RunPod bootstrap for samta-dashboard (minimize pod time) ==="
echo "GPUs will be used only for vLLM. All else overlapped."

# Detect if we can skip heavy vLLM (for openrouter/cloud harness testing)
SKIP_VLLM=${SKIP_VLLM:-0}
if [ "${HARNESS_CONTROLLER:-}" = "openrouter" ]; then
  SKIP_VLLM=1
  echo "HARNESS_CONTROLLER=openrouter detected → skipping local vLLM to save time"
fi

VLLM_MODEL=${VLLM_MODEL:-Qwen/Qwen2.5-7B-Instruct}
USE_VENV=${USE_VENV:-0}

START_TIME=$(date +%s)

# === Phase 1: Base deps (must be sequential, but quick) ===
echo "[1/parallel] Installing base system deps..."
apt-get update -qq
apt-get install -y -qq \
  git curl build-essential python3-pip python3-venv python3-dev \
  ca-certificates libssl-dev pkg-config \
  docker.io 2>/dev/null || true

# Start docker if needed
systemctl start docker || service docker start || true

echo "Base deps done."

# === Launch parallel background tasks ===
PIDS=()

# Rust (long download + install)
(
  echo "[parallel] Installing Rust..."
  if ! command -v cargo &> /dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  fi
  source "$HOME/.cargo/env"
  rustup update stable --no-self-update || true
  echo "Rust done."
) &
PIDS+=($!)

# Git clone (network)
(
  echo "[parallel] Cloning repo..."
  if [ ! -d "samta-dashboard" ]; then
    git clone --depth 1 https://github.com/samwarner16/samta-dashboard.git
  fi
  cd samta-dashboard
  git pull --ff-only || true
  echo "Clone done."
) &
PIDS+=($!)

# Docker pulls (network, independent)
(
  echo "[parallel] Pulling docker images..."
  docker pull -q postgres:15-alpine || true
  docker pull -q redis:7-alpine || true
  echo "Docker pulls done."
) &
PIDS+=($!)

# Python / vLLM prep (can start early)
if [ "$SKIP_VLLM" != "1" ]; then
  (
    echo "[parallel] Preparing Python / vLLM (heavy)..."
    if [ "$USE_VENV" = "1" ]; then
      python3 -m venv /opt/venv
      source /opt/venv/bin/activate
      PIP="pip"
    else
      PIP="pip3"
    fi
    $PIP install --upgrade -q pip wheel
    # Install torch + vllm for cu124 (matches common RunPod driver 12.8)
    # This is the long step - overlaps with clone/rust/docker
    $PIP install --upgrade --extra-index-url https://download.pytorch.org/whl/cu124 \
      torch torchvision torchaudio --quiet || true
    $PIP install vllm --extra-index-url https://download.pytorch.org/whl/cu124 --quiet || true
    echo "Python/vLLM prep done."
  ) &
  PIDS+=($!)
fi

# Wait for all parallel prep
echo "Waiting for parallel prep tasks..."
for pid in "${PIDS[@]}"; do
  wait "$pid" || echo "Warning: one parallel task had issues (continuing)"
done

echo "All parallel prep finished."

# === Sequential but now fast because overlapped ===
cd samta-dashboard || { echo "Clone failed"; exit 1; }

echo "[post-parallel] Building Rust worker (release)..."
cargo build --release -p application --bin worker || { echo "Build failed"; exit 1; }
echo "Worker binary ready: target/release/worker"

# Start vLLM if not skipped
if [ "$SKIP_VLLM" != "1" ]; then
  if ! curl -s --max-time 5 http://localhost:8000/v1/models > /dev/null 2>&1; then
    echo "Starting vLLM with $VLLM_MODEL (background)..."
    if [ "$USE_VENV" = "1" ]; then
      source /opt/venv/bin/activate
    fi
    nohup python -m vllm.entrypoints.openai.api_server \
      --model "$VLLM_MODEL" \
      --port 8000 \
      --gpu-memory-utilization 0.8 \
      --max-model-len 16384 \
      --trust-remote-code \
      > /tmp/vllm.log 2>&1 &
    VLLM_PID=$!
    echo "vLLM PID $VLLM_PID starting... (tail /tmp/vllm.log)"
    # Give it a head start but don't block too long
    sleep 15
  fi
else
  echo "Skipping vLLM (openrouter mode or SKIP_VLLM=1)"
fi

# Start DBs and API (docker compose or manual)
echo "Starting supporting stack (postgres/redis/api)..."
if command -v docker &> /dev/null && [ -f docker-compose.yml ]; then
  docker compose up -d postgres redis api || {
    echo "compose failed, falling back to manual docker run"
    docker run -d --name pg -e POSTGRES_USER=user -e POSTGRES_PASSWORD=pass -e POSTGRES_DB=agents_db -p 5432:5432 postgres:15-alpine || true
    docker run -d --name redis -p 6379:6379 redis:7-alpine || true
    nohup ./target/release/api > /tmp/api.log 2>&1 & || true
  }
else
  docker run -d --name pg -e POSTGRES_USER=user -e POSTGRES_PASSWORD=pass -e POSTGRES_DB=agents_db -p 5432:5432 postgres:15-alpine || true
  docker run -d --name redis -p 6379:6379 redis:7-alpine || true
  nohup ./target/release/api > /tmp/api.log 2>&1 & || true
fi
sleep 8

# Optional: start dashboard UI
nohup python3 -m http.server 4173 --directory crates/dashboard/frontend > /tmp/dash.log 2>&1 & || true

END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))
echo ""
echo "=== Bootstrap complete in ${ELAPSED}s (parallelized) ==="
echo "Ready to test the harness:"

if [ "$SKIP_VLLM" = "1" ]; then
  echo "  (openrouter mode - set your OPENROUTER_API_KEY and HARNESS_CONTROLLER=openrouter)"
  echo "  export HARNESS_CONTROLLER=openrouter"
  echo "  export OPENROUTER_API_KEY=sk-or-..."
  echo "  export HARNESS_OPENROUTER_MODEL=qwen/qwen-2.5-32b-instruct"
else
  echo "  export HARNESS_CONTROLLER=local_llm"
  echo "  export HARNESS_LLM_BASE_URL=http://localhost:8000/v1"
  echo "  export HARNESS_LLM_MODEL=$VLLM_MODEL"
fi

echo "  export DATABASE_URL=postgres://user:pass@localhost:5432/agents_db"
echo "  export REDIS_URL=redis://localhost:6379/"
echo "  ./target/release/worker"
echo ""
echo "Or: ./scripts/run-agent-harness.sh start 1"
echo "Monitor: tail -f /tmp/vllm.log /tmp/api.log"
echo "Dashboard: http://<pod-ip>:4173 (if ports exposed)"
echo ""
echo "To test with OpenRouter (cloud LLM for harness, no local vLLM needed):"
echo "  export HARNESS_CONTROLLER=openrouter"
echo "  export OPENROUTER_API_KEY=your-key"
echo "  ./target/release/worker"
echo ""
echo "Happy testing! (pod time minimized by parallel prep)"