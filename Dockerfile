FROM rust:1.90.0-bookworm AS builder

WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY static ./static
COPY migrations ./migrations

RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime

ARG VERSION=unknown
ARG REVISION=unknown
ARG CREATED=unknown

LABEL org.opencontainers.image.title="PageRune" \
      org.opencontainers.image.description="Self-hosted website update subscriptions" \
      org.opencontainers.image.version="$VERSION" \
      org.opencontainers.image.revision="$REVISION" \
      org.opencontainers.image.created="$CREATED"

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && groupadd --gid 10001 pagerune \
    && useradd --uid 10001 --gid pagerune --create-home --shell /usr/sbin/nologin pagerune \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder --chown=pagerune:pagerune /src/target/release/pagerune /usr/local/bin/pagerune
COPY --from=builder --chown=pagerune:pagerune /src/templates ./templates
COPY --from=builder --chown=pagerune:pagerune /src/static ./static
COPY --from=builder --chown=pagerune:pagerune /src/migrations ./migrations

USER pagerune:pagerune

EXPOSE 8080

ENTRYPOINT ["pagerune"]
