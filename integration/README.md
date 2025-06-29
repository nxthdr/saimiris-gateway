# Integration tests for saimiris-gateway

This integration setup launches a Saimiris Gateway and a Saimiris Agent in Docker containers. The agent registers itself with the gateway using a config file.

## Usage

* Start the environment:

```sh
docker compose up -d --force-recreate --renew-anon-volumes
```

* (Optional) Check logs:

```sh
docker compose logs -f
```

* Run integration tests:

```sh
./tests/test_database_integration.sh
```

* Stop the environment:

```sh
docker compose down
```
