FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
RUN apt-get update && apt-get install -y libgdal-dev clang libclang1
WORKDIR /app

FROM chef AS planner
COPY ./src /app/src
COPY ./migration /app/migration
COPY Cargo.lock Cargo.toml /app/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json

RUN cargo chef cook --release --recipe-path recipe.json

# Build application
COPY ./src /app/src
COPY ./migration /app/migration
COPY Cargo.lock Cargo.toml /app/

RUN cargo build --release --bin drop4crop-api

# We do not need the Rust toolchain to run the binary!
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y libgdal-dev
WORKDIR /app
COPY --from=builder /app/target/release/drop4crop-api /usr/local/bin
ENTRYPOINT ["/usr/local/bin/drop4crop-api"]
