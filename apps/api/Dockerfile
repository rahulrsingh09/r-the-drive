# ── Build stage ─────────────────────────────────────────────────────────────
FROM rust:1.97.1-slim@sha256:754a8924e308fb20a327febeda1a07053a2b0fd7474b5ac1cc460a6d33ab18f3 AS builder

WORKDIR /app
ENV SQLX_OFFLINE=true

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY . .
# Compile the real crate once so manifest targets cannot outgrow a dummy source tree.
RUN cargo build --locked --release --jobs 2

# ── Runtime stage ────────────────────────────────────────────────────────────
FROM postgres:18.4-bookworm@sha256:1961f96e6029a02c3812d7cb329a3b03a3ac2bb067058dec17b0f5596aca9296 AS runtime

LABEL org.opencontainers.image.source="https://github.com/your-org/r-the-drive"

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 curl && rm -rf /var/lib/apt/lists/*

# Non-root user with a dedicated group so the process does not run with GID 0.
RUN groupadd -r -g 1001 panda && useradd -r -u 1001 -g 1001 panda
USER 1001

WORKDIR /app
COPY --from=builder /app/target/release/panda-api ./panda-api
COPY --from=builder /app/migrations ./migrations

EXPOSE 3001
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3001/health || exit 1

ENTRYPOINT ["./panda-api"]
