# Shinrai-Trade documentation

Operational and reference docs for the workspace. Start with [Getting started](getting-started.md) if you are new to the repo.

| Doc | Contents |
|---|---|
| [Architecture](architecture.md) | Tech stack, crate layout, domain boundaries |
| [Getting started](getting-started.md) | Prerequisites, Postgres/NATS, build, run |
| [Configuration](configuration.md) | Environment variables |
| [Endpoints](endpoints.md) | HTTP / WebSocket API reference |
| [Local demo](demo.md) | Smoke tests, WebSocket subscribe, demo knobs |
| [Testing](testing.md) | Unit, integration, and CI test commands |
| [Milestones](milestones.md) | Delivery phases (1–5) and known gaps |
| [Milestones backlog](milestones-backlog.md) | 800+ code-derived improvement IDs |
| [Deployment](deployment.md) | Compose, secrets, Caddy TLS, hardening |

Deploy assets live under `deploy/` (`Caddyfile`, `secrets/`, `scripts/init-secrets.*`, Docker entrypoint).

Private design notes (`docs/design.md`, `docs/objectives-expanded.md`) are gitignored and are not part of this set.
