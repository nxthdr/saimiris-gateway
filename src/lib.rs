pub mod agent;
pub mod database;
pub mod jwt;
pub mod kafka;
pub mod probe;
pub mod probe_capnp;

use axum::{
    Router,
    extract::{Extension, Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Json,
    response::Response,
    routing::{get, post},
};
use rdkafka::message::{Header, OwnedHeaders};
use tower_http::trace::TraceLayer;
use tracing::{debug, error, warn};

use agent::{Agent, AgentConfig, AgentStore, HealthStatus};
use database::Database;
use probe::{SubmitProbesRequest, SubmitProbesResponse};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub agent_store: AgentStore,
    pub agent_key: String,
    pub kafka_config: kafka::KafkaConfig,
    pub kafka_producer: rdkafka::producer::FutureProducer,
    pub logto_jwks_uri: Option<String>,
    pub logto_issuer: Option<String>,
    pub bypass_jwt_validation: bool,
    pub database: Database,
}

// Client-facing API
pub fn create_client_app(state: AppState) -> Router {
    // Create a protected router for endpoints that require authentication
    let protected_routes = Router::new()
        .route("/user/usage", get(get_user_usage))
        .route("/probes", post(submit_probes))
        // .route("/admin/user-limit", post(set_user_limit))
        // .route("/admin/user-limit/:user_id", get(get_user_limit))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            jwt::jwt_middleware,
        ));

    Router::new()
        .route("/agents", get(list_agents))
        .route("/agent/{id}", get(get_agent))
        .route("/agent/{id}/config", get(get_agent_config))
        .route("/agent/{id}/health", get(get_agent_health))
        .merge(protected_routes)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

// Agent-facing API (API key required for agents to post their data)
pub fn create_agent_app(state: AppState) -> Router {
    Router::new()
        .route("/agent/register", post(register_agent))
        .route("/agent/{id}/config", post(update_agent_config))
        .route("/agent/{id}/health", post(update_agent_health))
        .with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            state,
            validate_agent_key,
        ))
        .layer(TraceLayer::new_for_http())
}

// Combined app with both client and agent endpoints
pub fn create_app(state: AppState) -> Router {
    let client_router = create_client_app(state.clone());
    let agent_router = create_agent_app(state);

    Router::new()
        .nest("/api", client_router)
        .nest("/agent-api", agent_router)
}

// API key validation middleware
async fn validate_agent_key(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    match auth_header {
        Some(key) if key == state.agent_key => Ok(next.run(request).await),
        _ => {
            warn!("Unauthorized access attempt to agent API");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

// Client-facing handlers (regular REST API)
async fn list_agents(State(state): State<AppState>) -> Json<Vec<Agent>> {
    let agents = state.agent_store.list_all().await;
    Json(agents)
}

async fn get_user_usage(
    Extension(auth_info): Extension<jwt::AuthInfo>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let user_identifier = &auth_info.sub;
    match state
        .database
        .get_user_usage_stats(user_identifier, None, None)
        .await
    {
        Ok(stats) => Ok(Json(serde_json::json!({
            "submission_count": stats.submission_count,
            "last_submitted": stats.last_submitted,
            "used": stats.total_probes,
            "limit": stats.limit
        }))),
        Err(err) => {
            error!("Failed to get user usage stats: {}", err);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Agent>, StatusCode> {
    match state.agent_store.get(&id).await {
        Some(agent) => Ok(Json(agent)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_agent_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<AgentConfig>>, StatusCode> {
    let agent = state
        .agent_store
        .get(&id)
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
    let agent = state
        .agent_store
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    match &agent.health {
        Some(health) => Ok(Json(health.clone())),
        None => Err(StatusCode::NOT_FOUND),
    }
}

// Agent-facing handlers
#[derive(serde::Deserialize)]
struct RegisterAgentRequest {
    id: String,
    secret: String,
}

async fn register_agent(
    State(state): State<AppState>,
    Json(payload): Json<RegisterAgentRequest>,
) -> Result<Json<Agent>, StatusCode> {
    match state
        .agent_store
        .add_agent(payload.id.clone(), payload.secret.clone())
        .await
    {
        Ok(()) => {
            let agent = state.agent_store.get(&payload.id).await.unwrap();
            debug!("Agent '{}' registered successfully", agent.id);
            Ok(Json(agent))
        }
        Err(_) => Err(StatusCode::CONFLICT),
    }
}

async fn update_agent_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(config): Json<Vec<AgentConfig>>,
) -> Result<Json<Vec<AgentConfig>>, StatusCode> {
    // Verify agent exists
    if state.agent_store.get(&id).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    state.agent_store.update_config(&id, config.clone()).await;
    debug!("Config updated for agent {}", id);
    Ok(Json(config))
}

async fn update_agent_health(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(health): Json<HealthStatus>,
) -> Result<Json<HealthStatus>, StatusCode> {
    // Verify agent exists
    if state.agent_store.get(&id).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    state.agent_store.update_health(&id, health.clone()).await;
    debug!("Health updated for agent {}", id);
    Ok(Json(health))
}

// Handler for submitting probes
async fn submit_probes(
    Extension(auth_info): Extension<jwt::AuthInfo>,
    State(state): State<AppState>,
    Json(request): Json<SubmitProbesRequest>,
) -> Result<Json<SubmitProbesResponse>, StatusCode> {
    // Additional validation if needed (basic validation happens during deserialization)
    if request.probes.is_empty() {
        debug!("User {} submitted an empty probe list", auth_info.sub);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Validate the probes
    if let Err(validation_error) = probe::validate_probes(&request.probes) {
        debug!("Validation error: {}", validation_error);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Check if user can submit these probes (daily rate limiting)
    let can_submit = match state
        .database
        .can_user_submit_probes(&auth_info.sub, request.probes.len() as u32, None)
        .await
    {
        Ok(can_submit) => can_submit,
        Err(err) => {
            error!("Failed to check user probe limit: {}", err);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if !can_submit {
        debug!(
            "User {} exceeded daily probe limit, cannot submit {} probes",
            auth_info.sub,
            request.probes.len()
        );
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    // Generate a unique measurement ID
    let measurement_id = Uuid::new_v4().to_string();

    let mut assigned_agents = Vec::new();

    // Validate that requested agents exist
    for agent_meta in &request.metadata {
        if let Some(agent) = state.agent_store.get(&agent_meta.id).await {
            // Only include healthy agents
            if let Some(health) = &agent.health {
                if health.healthy {
                    assigned_agents.push(agent_meta.id.clone());
                }
            }
        }
    }

    // If no valid agents were found, return an error
    if assigned_agents.is_empty() {
        debug!("No valid agents found for user {}", auth_info.sub);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Directly deserialize and create probe batches (max 1MB per batch)
    let probe_batches = match probe::deserialize_probes_batch(&request.probes, 1_000_000) {
        Ok(batches) => batches,
        Err(err) => {
            error!("Failed to deserialize probe batch: {}", err);
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    // Construct headers based on metadata
    let mut headers = OwnedHeaders::new();
    for agent_meta in &request.metadata {
        let agent_src_ip = agent_meta.ip_address.as_ref().map(|ip| ip.to_string());
        headers = headers.insert(Header {
            key: &agent_meta.id,
            value: agent_src_ip.as_ref().map(|ip| ip.as_str()),
        });
    }

    // Send each batch to Kafka
    let topic = &state.kafka_config.topic;
    for batch in probe_batches {
        // Use the measurement ID as the message key
        match kafka::send_to_kafka(
            &state.kafka_producer,
            topic,
            &measurement_id,
            &batch,
            Some(headers.clone()),
        )
        .await
        {
            Ok(_) => debug!(
                "Successfully sent probe batch for measurement {} to Kafka topic {}",
                measurement_id, topic
            ),
            Err(err) => {
                error!("Failed to send probe batch to Kafka: {}", err);
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }

    debug!(
        "User {} submitted {} probes for measurement {}, assigned to {} agents",
        auth_info.sub,
        request.probes.len(),
        measurement_id,
        assigned_agents.len()
    );

    // Record probe usage in database
    let user_identifier = &auth_info.sub;
    if let Err(err) = state
        .database
        .record_probe_usage(user_identifier, request.probes.len() as i32)
        .await
    {
        error!("Failed to record probe usage in database: {}", err);
        // Don't fail the request if database recording fails
    }

    Ok(Json(SubmitProbesResponse {
        measurement_id,
        accepted_probes: request.probes.len(),
        assigned_agents,
    }))
}

// Admin handler for setting user limits
// #[derive(serde::Deserialize)]
// struct SetUserLimitRequest {
//     user_id: String,
//     limit: u32,
// }

// async fn set_user_limit(
//     Extension(_auth_info): Extension<jwt::AuthInfo>,
//     State(state): State<AppState>,
//     Json(request): Json<SetUserLimitRequest>,
// ) -> Result<Json<serde_json::Value>, StatusCode> {
//     // TODO: Add proper admin authorization check here
//     // For now, just check if user has admin scope or role

//     match state
//         .database
//         .set_user_limit(&request.user_id, request.limit)
//         .await
//     {
//         Ok(limit) => Ok(Json(serde_json::json!({
//             "user_hash": limit.user_hash,
//             "limit": limit.probe_limit,
//             "updated_at": limit.updated_at
//         }))),
//         Err(err) => {
//             error!("Failed to set user limit: {}", err);
//             Err(StatusCode::INTERNAL_SERVER_ERROR)
//         }
//     }
// }

// async fn get_user_limit(
//     Extension(_auth_info): Extension<jwt::AuthInfo>,
//     State(state): State<AppState>,
//     Path(user_id): Path<String>,
// ) -> Result<Json<serde_json::Value>, StatusCode> {
//     // TODO: Add proper admin authorization check here

//     match state.database.get_user_limit(&user_id).await {
//         Ok(Some(limit)) => Ok(Json(serde_json::json!({
//             "user_hash": limit.user_hash,
//             "limit": limit.probe_limit,
//             "created_at": limit.created_at,
//             "updated_at": limit.updated_at
//         }))),
//         Ok(None) => Ok(Json(serde_json::json!({
//             "limit": 10000,  // Default limit
//             "is_default": true
//         }))),
//         Err(err) => {
//             error!("Failed to get user limit: {}", err);
//             Err(StatusCode::INTERNAL_SERVER_ERROR)
//         }
//     }
// }
