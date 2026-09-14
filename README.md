# drop4crop API

The API for CHANGE's drop4crop project.

## Tests

The unit tests run on the host with `cargo test --lib`. The rest need Postgres and Redis, so run the
whole suite in a container, mounting the registry and target directories so a rerun does not rebuild
everything:

```bash
docker network create d4c-test
docker run -d --rm --name d4c-test-db --network d4c-test \
  -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=psql -e POSTGRES_DB=drop4crop_test \
  postgis/postgis:16-3.4
docker run -d --rm --name d4c-test-redis --network d4c-test redis:7-alpine

docker run --rm --network d4c-test -v "$(pwd)":/app -w /app \
  -v drop4crop-target:/app/target -v drop4crop-cargo:/usr/local/cargo/registry \
  -e TEST_DATABASE_URL=postgresql://postgres:psql@d4c-test-db:5432/drop4crop_test \
  -e TILE_CACHE_URI=redis://d4c-test-redis:6379/0 \
  rust:1.95.0-trixie bash -c \
  "apt-get update && apt-get install -y libgdal-dev clang libclang1 && cargo test -- --test-threads=1"
```

Every test shares one database and truncates it on setup, so the suite needs `--test-threads=1`.
Tests that need S3 or Redis skip themselves when neither is reachable (`skip_if_no_s3!`,
`skip_if_no_redis!`); the database is not optional.

Pushes and pull requests run the same suite through `.github/workflows/test.yml`.

## Logging

Without `RUST_LOG` the app runs at `info,sqlx=warn`. Per-tile logging is at debug, so set it
explicitly when working locally:

```bash
RUST_LOG=info,drop4crop_api=debug,sqlx=warn cargo run
```
