# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`saimiris-gateway` is the API gateway for nxthdr's Saimiris probing platform. It is an async Rust (tokio + `axum`) HTTP service that authenticates users via Auth0 JWTs, enforces per-user daily probe quotas, encodes probes as Cap'n Proto, and publishes them to Kafka for the measurement pipeline. State lives in PostgreSQL (`sqlx`) plus an in-memory store of live agents. Configured entirely via CLI flags (see `--help`), not env vars.

## Commands

```bash
cargo build                       # build (needs capnproto, libsasl2-dev, libssl-dev installed)
cargo test                        # run all tests (no external deps; uses mock DB + in-process axum)
cargo test <name>                 # run a single test by name substring
cargo bench                       # criterion benchmark (probes_benchmark)
cargo fmt && cargo clippy         # format + lint (expected, not enforced in CI)
cargo run -- --bypass-jwt --agent-key testkey   # run locally with JWT disabled
```

CI (`.github/workflows/cicd.yml`) runs `cargo check --locked` then `cargo test --locked --verbose`, then builds/pushes multi-arch Docker images (the Dockerfile uses cargo-chef and needs `libpcap-dev` too). Integration tests under `integration/` (docker compose: PostgreSQL + Redpanda + a real agent) are NOT run in CI.

## Architecture

The app is one axum `Router` (`create_app` in `lib.rs`) split into two auth domains by URL prefix; everything shares one cloneable `AppState` (agent store, Kafka producer, DB, JWT config).

- **`lib.rs`** — all HTTP handlers + the routing split. `/api/*` is the **client-facing API** (`create_client_app`): public agent-listing routes are open, but `/user/*`, `/probes`, `/measurement/{id}/status` sit behind `jwt::jwt_middleware`. `/agent-api/*` is the **agent-facing API** (`create_agent_app`), guarded by `validate_agent_key` (a single shared bearer key == `state.agent_key`). `submit_probes` is the core flow: validate probes → check the user's IP is inside their allocated prefix → check daily quota → mint a `measurement_id` (UUID) → write per-agent `measurement_tracking` rows → batch-encode to Cap'n Proto (≤1MB/batch) → send each batch to Kafka with per-agent JSON headers (`src_ip`, `measurement_id`, `end_of_measurement`). Also holds the user-prefix math (`calculate_user_prefix`, `validate_user_ipv6`: agent prefix + 32-bit user_id) and `hash_user_identifier`.
- **`jwt.rs`** — Auth0 JWT validation. Fetches JWKS from `--auth0-jwks-uri` and caches it in a global for 12h; supports RSA + EC keys; verifies issuer but **not** audience. The validated `AuthInfo` (notably `sub`) is injected into request extensions. `--bypass-jwt` short-circuits this with a dummy `sub = "test-user-id"` — dev only.
- **`agent.rs`** — `AgentStore`, an `Arc<RwLock<HashMap>>` of live agents (id, secret, `Vec<AgentConfig>` caracat settings, `HealthStatus`). Purely in-memory: agents register/heartbeat via the agent API; `main.rs` spawns a task that evicts agents with no health check in 10min (runs every 5min), and `list_agents`/probe submission only consider agents healthy within 10min.
- **`database.rs`** — `Database` wraps an enum `DatabaseImpl::Real(PgPool)` | `Mock(MockStorage)`. The Mock is an in-memory reimplementation used by every test (`Database::new_mock()`) so no PostgreSQL is needed. Tables: `measurement_tracking` (source of truth, one row per measurement×agent), `user_limits`, `user_id_mappings`. `record_probe_usage` is a deprecated no-op kept for compatibility — real usage is derived from `measurement_tracking`.
- **`kafka.rs`** — `rdkafka` `FutureProducer` factory + `send_to_kafka`. Supports `PLAINTEXT` and `SASL_PLAINTEXT` (SCRAM by default).
- **`probe.rs`** — JSON↔Cap'n Proto probe conversion and validation. Wire format is a JSON array `[dst_addr, src_port, dst_port, ttl, protocol]`. Submission validation is **stricter than the type allows**: only IPv6 destinations and only `udp`/`icmpv6` are accepted (TCP/ICMP/IPv4 exist in the enum but are rejected by `validate_json_probe`). Ports/TTL must be non-zero.

## Conventions specific to this repo

- **`src/probe_capnp.rs` is generated** by `build.rs` from `schemas/probe.capnp` on every `cargo build` — never hand-edit it. Change the schema, not the generated file.
- **Handler request/response types are declared inline** in `lib.rs` next to their handler (e.g. `RegisterAgentRequest`), not in a shared models module. Cross-module types live in `probe.rs` / `agent.rs` / `database.rs`.
- **Errors to clients are `(StatusCode, Json<serde_json::Value>)`** shaped `{ "error": <code>, "message": ... }`; library code uses `anyhow`/`sqlx::Error`. Many handlers deliberately log-and-continue on non-critical DB failures rather than failing the request.
- **Every new DB method must be implemented for both `Real` and `Mock`** arms or tests can't exercise it — keep the Mock aggregation logic mirroring the SQL (the Mock manually reproduces the `measurement_status` view).
- **Tests** use `axum-test` against an in-process app built from `tests/common.rs::create_test_app_state()` (mock DB, dummy Kafka producer); flip `bypass_jwt_validation` to exercise authed routes without real tokens.

## Domain notes

- **`measurement_status` is a PostgreSQL VIEW**, not a table (defined in `migrations/20250718000001_create_measurement_tracking.sql`) — it aggregates `measurement_tracking` per measurement (total/completed agents, sent vs expected probes, `measurement_complete`). The Mock DB recomputes the same aggregation in Rust.
- **Migrations run automatically on startup** via `sqlx::migrate!("./migrations")` in `Database::initialize()`; adding a `migrations/*.sql` file is how you evolve the schema.
- **User identities are SHA-256 hashed** (`hash_user_identifier`, no salt — kept consistent with existing rows). The hash is the DB key (`user_hash`); separately, a stable 32-bit `user_id` per user (`user_id_mappings`) carves a per-user slice out of each agent's IPv6 prefix, which both gates source-IP validation on submit and is reported by `/user/prefixes`.
- **Measurement lifecycle**: the gateway creates tracking rows at submit time with `sent_probes=0`; agents later POST `/agent-api/agent/{id}/measurement/{mid}/status` to report progress/completion (looked up by `measurement_id`+`agent_id`, since the agent doesn't know the `user_hash`). Quotas are per-day (`DEFAULT_PROBE_LIMIT` = 10,000) computed from `expected_probes` since start-of-UTC-day.
