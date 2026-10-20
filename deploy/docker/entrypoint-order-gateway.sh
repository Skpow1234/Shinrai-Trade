#!/bin/sh
# Load Docker/Compose file secrets into env, then exec the gateway.
# Secrets are optional: explicit SHINRAI_* env still wins.
set -eu

read_secret() {
  _path="$1"
  if [ -f "$_path" ]; then
    # trim trailing newlines only
    tr -d '\r' <"$_path" | sed -e 's/[[:space:]]*$//'
  fi
}

if [ -z "${SHINRAI_DATABASE_URL:-}" ]; then
  _pw_file="${SHINRAI_DATABASE_PASSWORD_FILE:-/run/secrets/postgres_password}"
  _pw="$(read_secret "$_pw_file" || true)"
  if [ -n "${_pw:-}" ]; then
    _user="${SHINRAI_DATABASE_USER:-shinrai}"
    _host="${SHINRAI_DATABASE_HOST:-postgres}"
    _port="${SHINRAI_DATABASE_PORT:-5432}"
    _db="${SHINRAI_DATABASE_NAME:-shinrai}"
    # Passwords from init-secrets are URL-safe ([A-Za-z0-9_-]).
    export SHINRAI_DATABASE_URL="postgres://${_user}:${_pw}@${_host}:${_port}/${_db}"
  fi
fi

if [ -z "${SHINRAI_OG_OPS_TOKEN:-}" ]; then
  _ops_file="${SHINRAI_OG_OPS_TOKEN_FILE:-/run/secrets/ops_token}"
  _ops="$(read_secret "$_ops_file" || true)"
  if [ -n "${_ops:-}" ]; then
    export SHINRAI_OG_OPS_TOKEN="$_ops"
  fi
fi

exec shinrai-order-gateway "$@"
