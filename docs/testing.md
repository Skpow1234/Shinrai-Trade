# Testing

Same flags as CI:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Format in place:

```bash
cargo fmt --all
```

## Single crate / filter

```bash
cargo test -p shinrai-paper --all-features
cargo test -p shinrai-md-gateway --test auth
cargo test -p shinrai-order-gateway --test orders
cargo test -p shinrai-order-gateway --test portfolio
cargo test -p shinrai-order-gateway --test live_marks
cargo test -p shinrai-store --all-features
cargo test -p shinrai-audit --lib
cargo test -p shinrai-risk --lib
cargo test -p shinrai-md-gateway --test health
cargo test -p shinrai-md-fanout --lib overflow_drops_oldest
cargo test -p shinrai-paper --test proptest_invariants
```

## Postgres-backed tests

```bash
docker compose up -d postgres

SHINRAI_DATABASE_URL=postgres://shinrai:shinrai@127.0.0.1:5432/shinrai \
  cargo test -p shinrai-store --all-features

SHINRAI_DATABASE_URL=postgres://shinrai:shinrai@127.0.0.1:5432/shinrai \
  cargo test -p shinrai-order-gateway --all-features --test dual_write -- --test-threads=1
```

CI matrix `cargo test --workspace` is host-only. A dedicated **`test-db`** job runs store + dual-write against a Postgres service.

## Network / ignored tests

CI does **not** open a live Coinbase socket. Vendor tests use recorded JSON under `crates/protocols/market-data/tests/fixtures/`. Live portfolio marks are covered by `live_marks` (local MD gateway on `127.0.0.1:0`, no external network).

Optional ignored live smokes (require network / vendor keys as applicable):

```bash
cargo test -p shinrai-md-gateway --test coinbase_live -- --ignored
SHINRAI_MD_ALPACA_KEY=… SHINRAI_MD_ALPACA_SECRET=… \
  cargo test -p shinrai-md-gateway --test alpaca_md_live -- --ignored --nocapture
# Alpaca order venue / remote REST live tests: protocols/execution, order-gateway
```
