#!/usr/bin/env bash
# Starts everything needed for a local demo:
#   PostgreSQL check, Ollama, the dashboard build, hermit serve, and (optionally) a
#   smee.io tunnel that forwards GitHub webhooks to this machine.
# Usage: scripts/dev.sh            # local only
#        SMEE_URL=https://smee.io/xyz scripts/dev.sh   # + GitHub webhooks
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] && set -a && . ./.env && set +a
export DATABASE_URL="${DATABASE_URL:-postgres://localhost/hermit}"

pg_isready -q || { echo "PostgreSQL is not running (brew services start postgresql@16)"; exit 1; }
psql -lqt | cut -d'|' -f1 | grep -qw hermit || createdb hermit
curl -sf http://127.0.0.1:11434/api/tags >/dev/null || { echo "starting ollama"; (ollama serve >/dev/null 2>&1 &); sleep 2; }

[ -d web/out ] || (cd web && npm ci && npm run build)
cargo build --release -p hermit-cli

pids=()
cleanup() { kill "${pids[@]}" 2>/dev/null || true; }
trap cleanup EXIT
if [ -n "${SMEE_URL:-}" ]; then
  npx --yes smee-client --url "$SMEE_URL" --target "http://${HERMIT_BIND:-127.0.0.1:8080}/webhooks/github" &
  pids+=($!)
fi
./target/release/hermit serve
