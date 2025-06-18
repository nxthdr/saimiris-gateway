pub mod agent;
pub mod mtls;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing::info;
use uuid::Uuid;

use agent::{Agent, AgentConfig, AgentStore, HealthStatus};
use mtls::MtlsClient;

#[derive(Clone)]
pub struct AppState {
    pub agent_store: AgentStore,
}

#[derive(Clone)]
pub struct AgentMtlsState {
    pub agent_store: AgentStore,
    pub mtls_client: Arc<MtlsClient>,
}

// Client-facing API (regular REST - no mTLS required)
pub fn create_client_app(state: AppState) -> Router {
    Router::new()
        .route("/agents", get(list_agents))
        .route("/agent/{id}", get(get_agent))
        .route("/agent/{id}/config", get(get_agent_config))
        .route("/agent/{id}/health", get(get_agent_health))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

// Agent-facing API (mTLS required for agents to post their data)
pub fn create_agent_app(state: AgentMtlsState) -> Router {
    Router::new()
        .route("/agent/register", post(register_agent))
        .route("/agent/{id}/config", post(update_agent_config))
        .route("/agent/{id}/health", post(update_agent_health))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

// Combined app with both client and agent endpoints
pub fn create_app(client_state: AppState, agent_state: AgentMtlsState) -> Router {
    let client_router = create_client_app(client_state);
    let agent_router = create_agent_app(agent_state);

    Router::new()
        .nest("/api", client_router)
        .nest("/agent-api", agent_router)
}

// Client-facing handlers (regular REST API)
async fn list_agents(State(state): State<AppState>) -> Json<Vec<Agent>> {
    let agents = state.agent_store.list_all().await;
    Json(agents)
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Agent>, StatusCode> {
    let agent_id = id.parse::<Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;

    match state.agent_store.get(&agent_id).await {
        Some(agent) => Ok(Json(agent)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_agent_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AgentConfig>, StatusCode> {
    let agent_id = id.parse::<Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;

    let agent = state
        .agent_store
        .get(&agent_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    match &agent.config {
        Some(config) => Ok(Json(config.clone())),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_agent_health(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<HealthStatus>, StatusCode> {
    let agent_id = id.parse::<Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;

    let agent = state
        .agent_store
        .get(&agent_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    match &agent.health {
        Some(health) => Ok(Json(health.clone())),
        None => Err(StatusCode::NOT_FOUND),
    }
}

// Agent-facing handlers (mTLS required)
#[derive(serde::Deserialize)]
struct RegisterAgentRequest {
    name: String,
    endpoint: String,
}

async fn register_agent(
    State(state): State<AgentMtlsState>,
    Json(payload): Json<RegisterAgentRequest>,
) -> Result<Json<Agent>, StatusCode> {
    let agent_id = state
        .agent_store
        .add_agent(payload.name, payload.endpoint)
        .await;

    match state.agent_store.get(&agent_id).await {
        Some(agent) => {
            info!("Agent {} registered successfully", agent_id);
            Ok(Json(agent))
        }
        None => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn update_agent_config(
    State(state): State<AgentMtlsState>,
    Path(id): Path<String>,
    Json(config): Json<AgentConfig>,
) -> Result<Json<AgentConfig>, StatusCode> {
    let agent_id = id.parse::<Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;

    // Verify agent exists
    if state.agent_store.get(&agent_id).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    state
        .agent_store
        .update_config(&agent_id, config.clone())
        .await;
    info!("Config updated for agent {}", agent_id);
    Ok(Json(config))
}

async fn update_agent_health(
    State(state): State<AgentMtlsState>,
    Path(id): Path<String>,
    Json(health): Json<HealthStatus>,
) -> Result<Json<HealthStatus>, StatusCode> {
    let agent_id = id.parse::<Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;

    // Verify agent exists
    if state.agent_store.get(&agent_id).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    state
        .agent_store
        .update_health(&agent_id, health.clone())
        .await;
    info!("Health updated for agent {}", agent_id);
    Ok(Json(health))
}
