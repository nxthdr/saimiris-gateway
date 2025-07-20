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
use hex;
use ipnet::Ipv6Net;
use rdkafka::message::{Header, OwnedHeaders};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv6Addr};
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
        .route("/user/me", get(get_user_info))
        .route("/user/prefixes", get(get_user_prefixes))
        .route("/probes", post(submit_probes))
        .route(
            "/measurement/{id}/status",
            get(get_measurement_status_handler),
        )
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
        .route(
            "/agent/{id}/measurement/{measurement_id}/status",
            post(update_measurement_status),
        )
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

async fn get_user_info(
    Extension(auth_info): Extension<jwt::AuthInfo>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_identifier = &auth_info.sub;

    // Get or create user ID from database
    let user_id = match get_or_create_user_id(&state.database, user_identifier).await {
        Ok(id) => {
            debug!(
                "Successfully retrieved/created user ID {} for user {}",
                id, user_identifier
            );
            id
        }
        Err(err) => {
            error!(
                "Failed to get/create user ID for {}: {}",
                user_identifier, err
            );
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": 500,
                    "message": "Failed to process user identification"
                })),
            ));
        }
    };

    match state
        .database
        .get_user_usage_stats(user_identifier, None, None)
        .await
    {
        Ok(stats) => Ok(Json(serde_json::json!({
            "user_id": user_id,
            "submission_count": stats.submission_count,
            "last_submitted": stats.last_submitted,
            "used": stats.total_probes,
            "limit": stats.limit
        }))),
        Err(err) => {
            error!("Failed to get user usage stats: {}", err);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": 500,
                    "message": "Failed to retrieve user information"
                })),
            ))
        }
    }
}

async fn get_user_prefixes(
    Extension(auth_info): Extension<jwt::AuthInfo>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_identifier = &auth_info.sub;
    let user_id = match get_or_create_user_id(&state.database, user_identifier).await {
        Ok(id) => {
            debug!(
                "Successfully retrieved/created user ID {} for user {} in get_user_prefixes",
                id, user_identifier
            );
            id
        }
        Err(e) => {
            error!(
                "Failed to get/create user ID for {} in get_user_prefixes: {}",
                user_identifier, e
            );
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": 500,
                    "message": format!("Failed to get user ID: {}", e)
                })),
            ));
        }
    };

    // Get all agents and their configurations
    let agents = state.agent_store.list_all().await;
    let mut agent_prefixes = Vec::new();

    for agent in agents {
        // Only include agents that have IPv6 prefixes configured
        if let Some(config) = &agent.config {
            let mut prefixes = Vec::new();

            for agent_config in config {
                if let Some(ref agent_prefix) = agent_config.src_ipv6_prefix {
                    // Calculate the user's prefix within this agent's prefix
                    if let Some(user_prefix) = calculate_user_prefix(agent_prefix, user_id) {
                        prefixes.push(serde_json::json!({
                            "agent_prefix": agent_prefix,
                            "user_prefix": user_prefix
                        }));
                    }
                }
            }

            // Only include agents that have at least one IPv6 prefix
            if !prefixes.is_empty() {
                agent_prefixes.push(serde_json::json!({
                    "agent_id": agent.id,
                    "prefixes": prefixes
                }));
            }
        }
    }

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "agents": agent_prefixes
    })))
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
) -> Result<Json<SubmitProbesResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Additional validation if needed (basic validation happens during deserialization)
    if request.probes.is_empty() {
        debug!("User {} submitted an empty probe list", auth_info.sub);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": 400,
                "message": "Probe list cannot be empty"
            })),
        ));
    }

    // Validate the probes
    if let Err(validation_error) = probe::validate_probes(&request.probes) {
        debug!("Validation error: {}", validation_error);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": 400,
                "message": format!("Probe validation failed: {}", validation_error)
            })),
        ));
    }

    // Validate that all agent metadata has IP addresses specified
    for agent_meta in &request.metadata {
        if agent_meta.ip_address.is_none() {
            debug!(
                "User {} did not provide IP address for agent {}",
                auth_info.sub, agent_meta.id
            );
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": 400,
                    "message": format!("IP address is required for agent '{}'", agent_meta.id)
                })),
            ));
        }
    }

    // Validate source IP addresses for user's allocated prefixes
    let user_id = match get_or_create_user_id(&state.database, &auth_info.sub).await {
        Ok(id) => id,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": 500,
                    "message": format!("Failed to get user ID: {}", e)
                })),
            ));
        }
    };
    for agent_meta in &request.metadata {
        // We already validated that ip_address is present above
        let ip_addr = agent_meta.ip_address.as_ref().unwrap();

        if let IpAddr::V6(ipv6_addr) = ip_addr {
            // Get the agent to check its IPv6 prefix
            if let Some(agent) = state.agent_store.get(&agent_meta.id).await {
                if let Some(config) = &agent.config {
                    // Check if any of the agent's configurations have an IPv6 prefix
                    let mut valid_ip = false;
                    for agent_config in config {
                        if let Some(ref prefix) = agent_config.src_ipv6_prefix {
                            if validate_user_ipv6(ipv6_addr, prefix, user_id) {
                                valid_ip = true;
                                break;
                            }
                        }
                    }
                    if !valid_ip {
                        debug!(
                            "User {} attempted to use IP {} which is not within their allocated prefix",
                            auth_info.sub, ip_addr
                        );
                        return Err((
                            StatusCode::FORBIDDEN,
                            Json(serde_json::json!({
                                "error": 403,
                                "message": format!("IP address {} is not within your allocated prefix", ip_addr)
                            })),
                        ));
                    }
                }
            }
        }
        // IPv4 addresses are allowed without prefix validation for now
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
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": 500,
                    "message": "Failed to check probe limit"
                })),
            ));
        }
    };

    if !can_submit {
        debug!(
            "User {} exceeded daily probe limit, cannot submit {} probes",
            auth_info.sub,
            request.probes.len()
        );
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": 429,
                "message": "Daily probe limit exceeded"
            })),
        ));
    }

    // Generate a unique measurement ID
    let measurement_id = Uuid::new_v4();

    let mut assigned_agents = Vec::new();

    // Validate that requested agents exist
    for agent_meta in &request.metadata {
        if let Some(agent) = state.agent_store.get(&agent_meta.id).await {
            // Only include healthy agents
            if let Some(health) = &agent.health {
                if health.healthy {
                    assigned_agents.push(agent_meta.clone());
                }
            }
        }
    }

    // If no valid agents were found, return an error
    if assigned_agents.is_empty() {
        debug!("No valid agents found for user {}", auth_info.sub);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": 400,
                "message": "No healthy agents found for the requested measurement"
            })),
        ));
    }

    // Directly deserialize and create probe batches (max 1MB per batch)
    let probe_batches = match probe::deserialize_probes_batch(&request.probes, 1_000_000) {
        Ok(batches) => batches,
        Err(err) => {
            error!("Failed to deserialize probe batch: {}", err);
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": 400,
                    "message": "Failed to process probe data"
                })),
            ));
        }
    };

    // Initialize measurement tracking in database
    let user_identifier = &auth_info.sub;
    let user_hash = crate::hash_user_identifier(user_identifier);
    for agent_meta in &assigned_agents {
        if let Err(err) = state
            .database
            .create_measurement_tracking(
                &user_hash,
                measurement_id,
                &agent_meta.id,
                request.probes.len() as i32,
            )
            .await
        {
            error!(
                "Failed to create measurement tracking for agent {}: {}",
                agent_meta.id, err
            );
            // Continue with other agents even if one fails
        }
    }

    // Send each batch to Kafka with proper headers
    let topic = &state.kafka_config.topic;
    let total_batches = probe_batches.len();

    for (batch_index, batch) in probe_batches.iter().enumerate() {
        // Construct headers for this specific batch
        let mut headers = OwnedHeaders::new();
        let is_last_batch = batch_index == total_batches - 1;

        for agent_meta in &request.metadata {
            // Create JSON header value to match saimiris agent expectations
            let agent_info_json = serde_json::json!({
                "src_ip": agent_meta.ip_address,
                "measurement_id": measurement_id.to_string(),
                "end_of_measurement": is_last_batch,
            });
            let agent_info_str = agent_info_json.to_string();

            headers = headers.insert(Header {
                key: &agent_meta.id,
                value: Some(&agent_info_str),
            });
        }

        // Use the measurement ID as the message key
        match kafka::send_to_kafka(
            &state.kafka_producer,
            topic,
            &measurement_id.to_string(),
            batch,
            Some(headers),
        )
        .await
        {
            Ok(_) => debug!(
                "Successfully sent probe batch {} of {} for measurement {} to Kafka topic {}",
                batch_index + 1,
                total_batches,
                measurement_id,
                topic
            ),
            Err(err) => {
                error!("Failed to send probe batch to Kafka: {}", err);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": 500,
                        "message": "Failed to send probe data to processing queue"
                    })),
                ));
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

    // Get the probe count for the measurement
    let total_probe_count = request.probes.len() * assigned_agents.len();

    // Record probe usage in database
    if let Err(err) = state
        .database
        .record_probe_usage(user_identifier, measurement_id, total_probe_count as i32)
        .await
    {
        error!("Failed to record probe usage in database: {}", err);
        // Don't fail the request if database recording fails
    }

    Ok(Json(SubmitProbesResponse {
        id: measurement_id.to_string(),
        probes: total_probe_count,
        agents: assigned_agents,
    }))
}

// Handler for getting measurement status (client-facing)
async fn get_measurement_status_handler(
    Extension(auth_info): Extension<jwt::AuthInfo>,
    State(state): State<AppState>,
    Path(measurement_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let measurement_uuid = match Uuid::parse_str(&measurement_id) {
        Ok(uuid) => uuid,
        Err(_) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": 400,
                    "message": "Invalid measurement ID format"
                })),
            ));
        }
    };

    let user_hash = crate::hash_user_identifier(&auth_info.sub);

    match state
        .database
        .get_measurement_status(measurement_uuid, &user_hash)
        .await
    {
        Ok(Some(status)) => {
            // Also get detailed tracking information
            let tracking = match state
                .database
                .get_measurement_tracking(measurement_uuid, &user_hash)
                .await
            {
                Ok(tracking) => tracking,
                Err(err) => {
                    error!("Failed to get measurement tracking details: {}", err);
                    Vec::new()
                }
            };

            let agents_detail: Vec<_> = tracking
                .into_iter()
                .map(|t| {
                    serde_json::json!({
                        "agent_id": t.agent_id,
                        "expected_probes": t.expected_probes,
                        "sent_probes": t.sent_probes,
                        "is_complete": t.is_complete,
                        "updated_at": t.updated_at
                    })
                })
                .collect();

            Ok(Json(serde_json::json!({
                "measurement_id": status.measurement_id,
                "total_agents": status.total_agents,
                "completed_agents": status.completed_agents,
                "total_expected_probes": status.total_expected_probes,
                "total_sent_probes": status.total_sent_probes,
                "measurement_complete": status.measurement_complete,
                "started_at": status.started_at,
                "last_updated": status.last_updated,
                "agents": agents_detail
            })))
        }
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": 404,
                "message": "Measurement not found"
            })),
        )),
        Err(err) => {
            error!("Failed to get measurement status: {}", err);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": 500,
                    "message": "Failed to retrieve measurement status"
                })),
            ))
        }
    }
}

// Handler for agents to update measurement status (agent-facing)
#[derive(serde::Deserialize)]
struct UpdateMeasurementStatusRequest {
    sent_probes: i32,
    is_complete: bool,
}

async fn update_measurement_status(
    State(state): State<AppState>,
    Path((agent_id, measurement_id)): Path<(String, String)>,
    Json(request): Json<UpdateMeasurementStatusRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let measurement_uuid = match Uuid::parse_str(&measurement_id) {
        Ok(uuid) => uuid,
        Err(_) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": 400,
                    "message": "Invalid measurement ID format"
                })),
            ));
        }
    };

    // We need to find the user_hash for this measurement by looking up the tracking entry
    let tracking_entry = match state
        .database
        .get_measurement_tracking_by_agent(measurement_uuid, &agent_id)
        .await
    {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": 404,
                    "message": "Measurement tracking entry not found for this agent"
                })),
            ));
        }
        Err(err) => {
            error!("Failed to get measurement tracking: {}", err);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": 500,
                    "message": "Failed to retrieve measurement tracking"
                })),
            ));
        }
    };

    let user_hash = &tracking_entry.user_hash;

    match state
        .database
        .update_measurement_probe_count(
            measurement_uuid,
            user_hash,
            &agent_id,
            request.sent_probes,
            request.is_complete,
        )
        .await
    {
        Ok(updated_tracking) => {
            debug!(
                "Updated measurement {} for agent {}: {} probes sent, complete: {}",
                measurement_id, agent_id, request.sent_probes, request.is_complete
            );

            Ok(Json(serde_json::json!({
                "measurement_id": measurement_id,
                "agent_id": agent_id,
                "sent_probes": updated_tracking.sent_probes,
                "is_complete": updated_tracking.is_complete,
                "updated_at": updated_tracking.updated_at
            })))
        }
        Err(err) => {
            error!("Failed to update measurement status: {}", err);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": 500,
                    "message": "Failed to update measurement status"
                })),
            ))
        }
    }
}

/// Compute a consistent hash for a user identifier
/// This is used for database storage and lookup
pub fn hash_user_identifier(user_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    // Note: No salt here to maintain consistency with existing database records
    hex::encode(hasher.finalize())
}

/// Get or create a 32-bit user ID for a user identifier
/// Uses database table to ensure uniqueness and avoid collisions
pub async fn get_or_create_user_id(
    database: &Database,
    user_identifier: &str,
) -> Result<u32, String> {
    let user_hash = hash_user_identifier(user_identifier);

    // First, try to get existing user ID
    match database.get_user_id_by_hash(&user_hash).await {
        Ok(Some(user_id)) => {
            tracing::debug!(
                "Found existing user ID {} for user {}",
                user_id,
                user_identifier
            );
            return Ok(user_id);
        }
        Ok(None) => {
            tracing::debug!("Creating new user ID for user {}", user_identifier);
        }
        Err(e) => {
            return Err(format!("Database error when fetching user ID: {}", e));
        }
    }

    // Generate a new unique user ID with collision handling
    let max_attempts = 100; // Prevent infinite loops
    for attempt in 0..max_attempts {
        // Start with deterministic generation for consistency, then use random for fallback
        let candidate_id = if attempt == 0 {
            generate_deterministic_user_id(&user_hash)
        } else {
            generate_random_user_id()
        };

        match database
            .create_user_id_mapping(&user_hash, candidate_id)
            .await
        {
            Ok(()) => {
                tracing::debug!(
                    "Created new user ID {} for user {} (attempt {})",
                    candidate_id,
                    user_identifier,
                    attempt + 1
                );
                return Ok(candidate_id);
            }
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("UNIQUE constraint") || error_msg.contains("duplicate") {
                    // Check if this was a user_hash collision (another request created the user)
                    // or a user_id collision (our ID wasn't unique)
                    if error_msg.contains("user_hash") {
                        // Another request created this user while we were processing
                        // Try to fetch the existing user ID
                        match database.get_user_id_by_hash(&user_hash).await {
                            Ok(Some(existing_user_id)) => {
                                tracing::debug!(
                                    "User {} was created by another request, using existing ID {}",
                                    user_identifier,
                                    existing_user_id
                                );
                                return Ok(existing_user_id);
                            }
                            Ok(None) => {
                                // This shouldn't happen, but retry if it does
                                tracing::warn!(
                                    "Race condition detected for user {}, retrying",
                                    user_identifier
                                );
                                continue;
                            }
                            Err(e) => {
                                return Err(format!(
                                    "Database error during conflict resolution: {}",
                                    e
                                ));
                            }
                        }
                    } else {
                        // user_id collision, try a different ID
                        tracing::debug!(
                            "User ID {} collision on attempt {}, retrying with new ID",
                            candidate_id,
                            attempt + 1
                        );
                        continue;
                    }
                } else {
                    return Err(format!("Database error: {}", e));
                }
            }
        }
    }

    Err(format!(
        "Failed to generate unique user ID after {} attempts",
        max_attempts
    ))
}

/// Generate a deterministic 32-bit user ID from the user hash
/// This ensures the same user identifier always gets the same user ID
fn generate_deterministic_user_id(user_hash: &str) -> u32 {
    // Use the first 8 characters of the hex hash to create a deterministic ID
    let hash_prefix = &user_hash[..8];
    let hash_u32 = u32::from_str_radix(hash_prefix, 16).unwrap_or(1000);

    // Ensure the ID is within our valid range
    const MIN_USER_ID: u32 = 1000;
    const MAX_USER_ID: u32 = 0xFFFF_FF00; // Leave some headroom

    if hash_u32 < MIN_USER_ID {
        MIN_USER_ID + (hash_u32 % (MAX_USER_ID - MIN_USER_ID))
    } else if hash_u32 > MAX_USER_ID {
        MIN_USER_ID + (hash_u32 % (MAX_USER_ID - MIN_USER_ID))
    } else {
        hash_u32
    }
}

/// Generate a random 32-bit user ID for fallback when deterministic generation collides
fn generate_random_user_id() -> u32 {
    use rand::Rng;
    let mut rng = rand::rng();

    // Generate a random 32-bit number within our valid range
    const MIN_USER_ID: u32 = 1000;
    const MAX_USER_ID: u32 = 0xFFFF_FF00; // Leave some headroom

    rng.random_range(MIN_USER_ID..=MAX_USER_ID)
}

/// Parse an agent's prefix and calculate the user's prefix address
/// Returns the IPv6 address of the user's prefix, or None if the agent prefix is invalid
/// The user gets 32 bits of space for their allocation within the agent's prefix
fn calculate_user_prefix_addr(agent_prefix: &str, user_id: u32) -> Option<Ipv6Addr> {
    let agent_net: Ipv6Net = agent_prefix.parse().ok()?;

    // Validate there's enough space for a 32-bit user ID
    if agent_net.prefix_len() > 96 {
        return None;
    }

    // Get the network address and add the user ID
    let network_addr = agent_net.network();
    let network_u128 = u128::from(network_addr);

    // Calculate how many bits to shift the user ID
    let user_id_shift = 128 - agent_net.prefix_len() - 32;
    let user_id_bits = (user_id as u128) << user_id_shift;

    // Combine network address with user ID
    let user_prefix_addr = network_u128 | user_id_bits;

    Some(Ipv6Addr::from(user_prefix_addr))
}

/// Validate that an IPv6 address is within the user's allocated prefix
/// Agent prefix + User ID (32 bits) = user prefix
pub fn validate_user_ipv6(user_ip: &Ipv6Addr, agent_prefix: &str, user_id: u32) -> bool {
    let agent_net: Ipv6Net = match agent_prefix.parse() {
        Ok(net) => net,
        Err(_) => return false,
    };

    // Validate there's enough space for a 32-bit user ID
    if agent_net.prefix_len() > 96 {
        return false;
    }

    // Calculate the user's prefix network
    let user_prefix_addr = match calculate_user_prefix_addr(agent_prefix, user_id) {
        Some(addr) => addr,
        None => return false,
    };

    // Create the user's prefix network (agent prefix length + 32 bits)
    let user_prefix_len = agent_net.prefix_len() + 32;
    let user_net = match Ipv6Net::new(user_prefix_addr, user_prefix_len) {
        Ok(net) => net,
        Err(_) => return false,
    };

    // Check if the user's IP is within their allocated prefix
    user_net.contains(user_ip)
}

/// Calculate the user's prefix within an agent's prefix
/// Returns the user's allocated prefix as a string, or None if the agent prefix is invalid
/// The user gets 32 bits of space within the agent's prefix
pub fn calculate_user_prefix(agent_prefix: &str, user_id: u32) -> Option<String> {
    let agent_net: Ipv6Net = agent_prefix.parse().ok()?;

    // Validate there's enough space for a 32-bit user ID
    if agent_net.prefix_len() > 96 {
        return None;
    }

    let user_prefix_addr = calculate_user_prefix_addr(agent_prefix, user_id)?;
    let user_prefix_len = agent_net.prefix_len() + 32; // User gets 32 bits within agent prefix

    Some(format!("{}/{}", user_prefix_addr, user_prefix_len))
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
