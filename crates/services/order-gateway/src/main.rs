//! Order gateway process.

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::http::Request;
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use shinrai_messaging::{connect_nats, EventSink, LogSink};
use shinrai_order_gateway::{router, AppState, GatewayConfig};
use shinrai_store::{connect_from_env, migrate, StoreError};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    let dev = args.iter().any(|a| a == "--dev") || env_truthy("SHINRAI_OG_DEV");
    if dev {
        // So GatewayConfig::from_env also sees it (and scripts can rely on either).
        env::set_var("SHINRAI_OG_DEV", "1");
    }

    let _telemetry = shinrai_telemetry::init("shinrai-order-gateway")?;

    let bind = env::var("SHINRAI_OG_BIND").unwrap_or_else(|_| "127.0.0.1:8081".into());
    let mut config = GatewayConfig::from_env();
    if dev {
        config.apply_paper_dev_defaults();
    }
    let mut state = AppState::from_config(&config);

    match connect_from_env().await {
        Ok(pool) => {
            if env_truthy("SHINRAI_RUN_MIGRATIONS") {
                migrate(&pool).await?;
                eprintln!("shinrai-order-gateway: migrations applied");
            }
            if shinrai_store::has_durable_state(&pool).await? {
                state.hydrate_from_store(pool).await?;
                eprintln!("shinrai-order-gateway: hydrated PaperEngine from Postgres");
            } else {
                state.attach_store(pool).await;
                state.persist_bootstrap().await?;
                eprintln!(
                    "shinrai-order-gateway: Postgres write-through enabled (fresh bootstrap)"
                );
            }
            if let Some(pool) = state.store_pool() {
                let metrics = state.outbox_metrics();
                let poll_ms: u64 = env::var("SHINRAI_OUTBOX_POLL_MS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1_000);
                let sink = build_event_sink().await?;
                let kind = sink.kind();
                tokio::spawn(shinrai_order_gateway::run_outbox_publisher(
                    pool,
                    metrics,
                    sink,
                    Duration::from_millis(poll_ms),
                ));
                eprintln!(
                    "shinrai-order-gateway: outbox publisher started (poll {poll_ms}ms, sink={kind:?})"
                );
            }
        }
        Err(StoreError::MissingDatabaseUrl) => {
            eprintln!("shinrai-order-gateway: no SHINRAI_DATABASE_URL; in-memory only");
        }
        Err(err) => return Err(err.into()),
    }

    let app = router(state).layer(from_fn(inject_real_ip));
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!("shinrai-order-gateway listening on {bind}");
    if dev {
        print_dev_banner(&bind);
    }
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn print_help() {
    eprintln!(
        "\
shinrai-order-gateway — paper order HTTP gateway

Usage:
  cargo run -p shinrai-order-gateway -- [--dev] [--help]

Options:
  --dev    Paper-dev defaults (token=dev, trader@account 1, $10k, AAPL mark)
           Same as SHINRAI_OG_DEV=1. Explicit SHINRAI_OG_* env still wins.
  --help   Show this message

Quick start (UI):
  cargo run -p shinrai-order-gateway -- --dev
  open http://127.0.0.1:8081/ui   # token: dev

Or:  ./scripts/dev-ui.sh   /   .\\scripts\\dev-ui.ps1
"
    );
}

fn print_dev_banner(bind: &str) {
    let host = if bind.starts_with("0.0.0.0:") {
        format!("127.0.0.1:{}", bind.trim_start_matches("0.0.0.0:"))
    } else {
        bind.to_string()
    };
    eprintln!(
        "\
┌─────────────────────────────────────────────────────────┐
│  Paper trader UI  http://{host}/ui
│  Token            dev
│  Or credentials   client_id=dev  secret=s3cret
│  Ops dashboard    http://{host}/v1/ops
└─────────────────────────────────────────────────────────┘"
    );
}

async fn build_event_sink()
-> Result<Arc<dyn EventSink>, Box<dyn std::error::Error + Send + Sync>> {
    let url = env::var("SHINRAI_NATS_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match url {
        Some(url) => {
            let prefix = env::var("SHINRAI_NATS_SUBJECT_PREFIX")
                .unwrap_or_else(|_| "shinrai".into());
            let sink = connect_nats(&url, prefix).await?;
            Ok(Arc::new(sink) as Arc<dyn EventSink>)
        }
        None => Ok(Arc::new(LogSink) as Arc<dyn EventSink>),
    }
}

/// Sets `X-Real-IP` from the TCP peer when not already present (ops allowlist).
async fn inject_real_ip(mut req: Request<axum::body::Body>, next: Next) -> Response {
    if req.headers().get("x-real-ip").is_none() {
        if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
            if let Ok(value) = addr.ip().to_string().parse() {
                req.headers_mut().insert("x-real-ip", value);
            }
        }
    }
    next.run(req).await
}

fn env_truthy(key: &str) -> bool {
    matches!(
        env::var(key).ok().as_deref().map(str::trim),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}
