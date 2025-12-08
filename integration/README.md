# Integration tests for saimiris-gateway

This integration setup tests the Saimiris Gateway with a Saimiris Agent running in **dry-run mode**.

## Quick Start

Start the environment:

```sh
docker compose up -d --force-recreate --renew-anon-volumes
```

Wait a few seconds for services to start, then run the test:

```sh
./tests/test_probe_pipeline.sh
```

Stop the environment:

```sh
docker compose down
```

## What gets tested

The integration test verifies the gateway's probe handling pipeline:

1. **Gateway API** - REST endpoints responding correctly
2. **Agent registration** - Agent successfully registers with gateway
3. **Probe submission** - Probes validated and accepted via API
4. **Kafka delivery** - Probes serialized and sent to Kafka
5. **Agent processing** - Agent receives and processes probes from Kafka
6. **Usage tracking** - Database correctly tracks submissions and usage limits

## Dry-run mode

The agent runs with `dry_run: true` in the Caracat configuration. This means:
- ✅ All probe processing logic is executed
- ✅ Probes are received, validated, and queued
- ❌ No actual network probes are transmitted

This is intentional - the integration test focuses on verifying the **gateway's functionality**, not the agent's network probing capabilities.
