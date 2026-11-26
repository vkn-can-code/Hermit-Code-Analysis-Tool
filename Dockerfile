# Multi-stage image: Rust binary + dashboard + Semgrep. Ollama and PostgreSQL run as
# separate services (see docker-compose.yml).
FROM rust:1-bookworm AS rust
WORKDIR /src
COPY . .
RUN cargo build --release -p hermit-cli

FROM node:22-bookworm AS web
WORKDIR /web
COPY web/package*.json ./
RUN npm ci
COPY web/ .
RUN npm run build

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates python3-pip pipx \
    && pipx install semgrep && rm -rf /var/lib/apt/lists/*
ENV PATH="/root/.local/bin:${PATH}" HERMIT_BIND=0.0.0.0:8080 HERMIT_WEB_DIR=/app/web
COPY --from=rust /src/target/release/hermit /usr/local/bin/hermit
COPY --from=web /web/out /app/web
EXPOSE 8080
CMD ["hermit", "serve"]
