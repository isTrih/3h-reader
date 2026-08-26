# syntax=docker/dockerfile:1.7

FROM rust:1.94-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
ARG TARGETPLATFORM

# Keep dependency compilation in a layer that is independent of application source changes.
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=cargo-target-${TARGETPLATFORM},target=/app/target \
    mkdir src \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf '' > src/lib.rs \
    && cargo build --locked --release \
    && rm -rf src

COPY src ./src

RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=cargo-target-${TARGETPLATFORM},target=/app/target \
    touch src/main.rs src/lib.rs \
    && cargo build --locked --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home app

COPY --from=builder /app/target/release/openlark-bitable-service /usr/local/bin/openlark-bitable-service

USER 10001

ENV BIND_ADDR=0.0.0.0:8080

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/openlark-bitable-service"]
