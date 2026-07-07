# Dockerfile for samta-dashboard
# Multi-stage build for smaller runtime image

FROM rust:1.80-bookworm AS builder

WORKDIR /app

# Copy manifests first for caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Build dependencies
RUN cargo build --release --workspace

# Copy source and build again (for changes)
COPY . .

RUN cargo build --release -p api -p application

# Runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binaries
COPY --from=builder /app/target/release/api /usr/local/bin/api
COPY --from=builder /app/target/release/worker /usr/local/bin/worker

# Copy necessary assets
COPY migrations ./migrations
COPY crates/dashboard/frontend ./crates/dashboard/frontend
COPY scripts ./scripts
COPY .env.example .env.example

# Default to worker, can override
CMD ["worker"]

# Expose ports (for reference, actual in compose or pod)
EXPOSE 8080 4173