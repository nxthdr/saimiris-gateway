# Saimiris Gateway

> [!WARNING]
> Currently in early-stage development.

The Saimiris Gateway is a web service that provides a REST API to interact with the Saimiris measurement pipeline.

## Features

- **Agent Management**: Register and manage measurement agents
- **Probe Submission**: Submit probes to send via the measurements pipeline while ensuring a given quota is respected
- **Auto-Migration**: Database schema is automatically created and updated on startup
- **Privacy-First**: User identifiers are SHA-256 hashed for privacy protection

## Getting Started

The gateway is configured via **command-line arguments** (not environment variables):

```bash
# Basic usage
./saimiris-gateway \
  --database-url "postgresql://user:password@localhost/saimiris_gateway" \
  --agent-key "your-secret-agent-key" \
  --kafka-brokers "localhost:9092" \
  --logto-jwks-uri "https://your-logto-instance.com/oidc/jwks" \
  --logto-issuer "https://your-logto-instance.com/oidc"

# Development mode (bypass JWT)
./saimiris-gateway \
  --database-url "postgresql://user:password@localhost/saimiris_gateway" \
  --agent-key "testkey" \
  --kafka-brokers "localhost:9092" \
  --bypass-jwt

# Show all available options
./saimiris-gateway --help
```

### Key Configuration Options

- `--address`: Server bind address (default: 0.0.0.0:8080)
- `--database-url`: PostgreSQL connection string (required)
- `--agent-key`: Authentication key for agents (required)
- `--kafka-brokers`: Kafka broker addresses (default: localhost:9092)
- `--logto-jwks-uri`: LogTo JWKS URI for JWT validation
- `--logto-issuer`: LogTo issuer for JWT validation
- `--bypass-jwt`: Bypass JWT validation (development only)

## API Endpoints

### Client API (requires JWT authentication)

- `GET /api/user/usage` - Get user probe daily usage statistics
- `POST /api/probes` - Submit probes for measurement

### Agent API (requires agent key)

- `POST /agent-api/agent/register` - Register a new agent
- `POST /agent-api/agent/{id}/config` - Update agent configuration
- `POST /agent-api/agent/{id}/health` - Update agent health status

### Public API

- `GET /api/agents` - List all agents
- `GET /api/agent/{id}` - Get agent details
- `GET /api/agent/{id}/config` - Get agent configuration
- `GET /api/agent/{id}/health` - Get agent health status

## Testing

The project has comprehensive tests that **don't require any external dependencies**:

```bash
# Run all tests (no database/Kafka required!)
cargo test
```

### Integration Testing

For full end-to-end testing with real PostgreSQL and Kafka:

```bash
cd integration
docker compose up -d
./tests/test_database_integration.sh
docker compose down
```

