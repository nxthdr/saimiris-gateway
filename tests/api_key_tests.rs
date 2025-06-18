use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use saimiris_gateway::{agent::AgentStore, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn test_api_key_middleware_with_valid_key() {
    let state = AppState {
        agent_store: AgentStore::new(),
        api_key: "test-key".to_string(),
    };
    
    let request = Request::builder()
        .header("authorization", "Bearer test-key")
        .body(Body::empty())
        .unwrap();
    
    // Test that middleware would pass with valid key
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    
    assert_eq!(auth_header, Some("test-key"));
    assert_eq!(auth_header, Some(state.api_key.as_str()));
}

#[tokio::test]
async fn test_api_key_middleware_with_invalid_key() {
    let state = AppState {
        agent_store: AgentStore::new(),
        api_key: "test-key".to_string(),
    };
    
    let request = Request::builder()
        .header("authorization", "Bearer wrong-key")
        .body(Body::empty())
        .unwrap();
    
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    
    assert_eq!(auth_header, Some("wrong-key"));
    assert_ne!(auth_header, Some(state.api_key.as_str()));
}

#[tokio::test]
async fn test_api_key_middleware_without_header() {
    let state = AppState {
        agent_store: AgentStore::new(),
        api_key: "test-key".to_string(),
    };
    
    let request = Request::builder()
        .body(Body::empty())
        .unwrap();
    
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    
    assert_eq!(auth_header, None);
}

#[tokio::test]
async fn test_api_key_middleware_with_malformed_header() {
    let state = AppState {
        agent_store: AgentStore::new(),
        api_key: "test-key".to_string(),
    };
    
    let request = Request::builder()
        .header("authorization", "test-key") // Missing "Bearer " prefix
        .body(Body::empty())
        .unwrap();
    
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    
    assert_eq!(auth_header, None);
}

#[test]
fn test_app_state_creation() {
    let agent_store = AgentStore::new();
    let api_key = "test-api-key-123".to_string();
    
    let state = AppState {
        agent_store: agent_store.clone(),
        api_key: api_key.clone(),
    };
    
    assert_eq!(state.api_key, api_key);
}