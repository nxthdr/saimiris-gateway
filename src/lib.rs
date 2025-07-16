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
                    // Calculate the user's /80 prefix within this agent's /48 prefix
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
                            "User {} attempted to use IP {} which is not within their allocated /80 prefix",
                            auth_info.sub, ip_addr
                        );
                        return Err((
                            StatusCode::FORBIDDEN,
                            Json(serde_json::json!({
                                "error": 403,
                                "message": format!("IP address {} is not within your allocated /80 prefix", ip_addr)
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

    // Construct headers based on metadata
    let mut headers = OwnedHeaders::new();
    for agent_meta in &request.metadata {
        // Create JSON header value to match saimiris agent expectations
        let agent_info_json = serde_json::json!({
            "src_ip": agent_meta.ip_address
        });
        let agent_info_str = agent_info_json.to_string();

        headers = headers.insert(Header {
            key: &agent_meta.id,
            value: Some(&agent_info_str),
        });
    }

    // Send each batch to Kafka
    let topic = &state.kafka_config.topic;
    for batch in probe_batches {
        // Use the measurement ID as the message key
        match kafka::send_to_kafka(
            &state.kafka_producer,
            topic,
            &measurement_id.to_string(),
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
    let user_identifier = &auth_info.sub;
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
        Ok(Some(user_id)) => return Ok(user_id),
        Ok(None) => {
            // User doesn't exist, continue to create new ID
        }
        Err(e) => return Err(format!("Database error: {}", e)),
    }

    // Generate a new unique user ID
    let max_attempts = 100; // Prevent infinite loops
    for attempt in 0..max_attempts {
        let candidate_id = generate_random_user_id();

        match database
            .create_user_id_mapping(&user_hash, candidate_id)
            .await
        {
            Ok(()) => return Ok(candidate_id),
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("UNIQUE constraint") || error_msg.contains("duplicate") {
                    // Check if this was a user_hash collision (another request created the user)
                    // or a user_id collision (our random ID wasn't unique)
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
                        // user_id collision, try a different random ID
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

/// Generate a random 32-bit user ID
/// Excludes reserved ranges and common problematic values
fn generate_random_user_id() -> u32 {
    use rand::Rng;
    let mut rng = rand::rng();

    // Generate random ID in range, excluding reserved values
    // Avoid 0, 0xFFFFFFFF, and low values that might be reserved
    const MIN_USER_ID: u32 = 1000;
    const MAX_USER_ID: u32 = 0xFFFF_FF00; // Leave some headroom

    rng.random_range(MIN_USER_ID..=MAX_USER_ID)
}

/// Parse an agent's /48 prefix and calculate the user's /80 prefix address
/// Returns the IPv6 address of the user's /80 prefix, or None if the agent prefix is invalid
fn calculate_user_prefix_addr(agent_prefix: &str, user_id: u32) -> Option<Ipv6Addr> {
    // Parse the agent's /48 prefix
    let agent_prefix_parts: Vec<&str> = agent_prefix.split('/').collect();
    if agent_prefix_parts.len() != 2 {
        return None;
    }

    let prefix_len: u8 = match agent_prefix_parts[1].parse() {
        Ok(len) if len == 48 => len,
        _ => return None,
    };

    // Validate it's a /48 prefix
    if prefix_len != 48 {
        return None;
    }

    let agent_base: Ipv6Addr = match agent_prefix_parts[0].parse() {
        Ok(addr) => addr,
        Err(_) => return None,
    };

    // Create the user's /80 prefix by combining agent /48 + user ID (32 bits)
    let agent_segments = agent_base.segments();
    let mut user_segments = agent_segments;

    // The user ID occupies the next 32 bits after the /48 prefix
    // /48 means first 3 segments (48 bits) are the agent prefix
    // Next 2 segments (32 bits) should contain the user ID
    user_segments[3] = ((user_id >> 16) & 0xFFFF) as u16;
    user_segments[4] = (user_id & 0xFFFF) as u16;

    Some(Ipv6Addr::from(user_segments))
}

/// Validate that an IPv6 address is within the user's allocated /80 prefix
/// Agent prefix (/48) + User ID (32 bits) = /80
pub fn validate_user_ipv6(user_ip: &Ipv6Addr, agent_prefix: &str, user_id: u32) -> bool {
    let expected_prefix_addr = match calculate_user_prefix_addr(agent_prefix, user_id) {
        Some(addr) => addr,
        None => return false,
    };

    let user_segments = user_ip.segments();
    let expected_segments = expected_prefix_addr.segments();

    // Check if the user's IP matches the expected /80 prefix
    // First 5 segments (80 bits) must match
    for i in 0..5 {
        if user_segments[i] != expected_segments[i] {
            return false;
        }
    }

    true
}

/// Calculate the user's /80 prefix within an agent's /48 prefix
/// Returns the user's allocated /80 prefix as a string, or None if the agent prefix is invalid
pub fn calculate_user_prefix(agent_prefix: &str, user_id: u32) -> Option<String> {
    let user_prefix_addr = calculate_user_prefix_addr(agent_prefix, user_id)?;
    Some(format!("{}/80", user_prefix_addr))
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
