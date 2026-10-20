#!/usr/bin/env bash
# Create deploy/secrets/{postgres_password,ops_token} if missing.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIR="$ROOT/deploy/secrets"
RANDOM_MODE=0

for arg in "$@"; do
  case "$arg" in
    --random|-r) RANDOM_MODE=1 ;;
    --help|-h)
      echo "Usage: $0 [--random]"
      exit 0
      ;;
  esac
done

mkdir -p "$DIR"

rand_token() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -base64 24 | tr -d '/+=' | tr -d '\n' | head -c 32
  else
    # Fallback: urandom hex
    head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n' | head -c 32
  fi
  echo
}

write_secret() {
  local name="$1"
  local default="$2"
  local path="$DIR/$name"
  if [[ -f "$path" ]]; then
    echo "keep  $path"
    return
  fi
  if [[ "$RANDOM_MODE" -eq 1 ]]; then
    rand_token >"$path"
  else
    printf '%s\n' "$default" >"$path"
  fi
  chmod 600 "$path" 2>/dev/null || true
  echo "wrote $path"
}

write_secret postgres_password shinrai
write_secret ops_token compose-ops

echo "Done. Start stack with:"
echo "  docker compose --profile full up -d --build"
echo "TLS (also run init-secrets first):"
echo "  docker compose --profile full --profile tls -f compose.yaml -f compose.tls.yaml up -d --build"
