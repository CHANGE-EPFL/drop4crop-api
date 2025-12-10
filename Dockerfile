FROM rust:1.90.0-bookworm AS chef

# Build GDAL 3.12 from source
RUN apt-get update && apt-get install -y \
    cmake \
    clang \
    libclang1 \
    libproj-dev \
    libsqlite3-dev \
    libtiff-dev \
    libcurl4-openssl-dev \
    libpng-dev \
    libjpeg-dev \
    libexpat1-dev \
    libgeos-dev \
    libssl-dev \
    pkg-config \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN git clone --depth 1 --branch v3.12.0 https://github.com/OSGeo/gdal.git /tmp/gdal && \
    cd /tmp/gdal && \
    mkdir build && cd build && \
    cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr/local && \
    make -j$(nproc) && \
    make install && \
    ldconfig && \
    rm -rf /tmp/gdal

RUN cargo install cargo-chef
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
RUN apt-get update && apt-get install -y \
    libproj25 \
    libsqlite3-0 \
    libtiff6 \
    libcurl4 \
    libpng16-16 \
    libjpeg62-turbo \
    libexpat1 \
    libgeos-c1v5 \
    && rm -rf /var/lib/apt/lists/*

# Copy GDAL libraries from builder
COPY --from=builder /usr/local/lib/libgdal* /usr/local/lib/
COPY --from=builder /usr/local/share/gdal /usr/local/share/gdal
RUN ldconfig

WORKDIR /app
COPY --from=builder /app/target/release/drop4crop-api /usr/local/bin
ENTRYPOINT ["/usr/local/bin/drop4crop-api"]
