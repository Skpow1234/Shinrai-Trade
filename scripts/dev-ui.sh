#!/usr/bin/env bash
# One-command paper trader: defaults + order gateway + print UI URL.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export SHINRAI_OG_DEV="${SHINRAI_OG_DEV:-1}"
export SHINRAI_OG_BIND="${SHINRAI_OG_BIND:-127.0.0.1:8081}"

UI="http://${SHINRAI_OG_BIND}/ui"
echo "Starting paper order-gateway (dev defaults)…"
echo "  UI:    $UI"
echo "  Token: dev"
echo

# Open browser after a short delay (best-effort; ignore failures).
(
  sleep 2
  if command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$UI" >/dev/null 2>&1 || true
  elif command -v open >/dev/null 2>&1; then
    open "$UI" >/dev/null 2>&1 || true
  elif command -v cmd.exe >/dev/null 2>&1; then
    cmd.exe /c start "" "$UI" >/dev/null 2>&1 || true
  fi
) &

exec cargo run -p shinrai-order-gateway -- --dev
