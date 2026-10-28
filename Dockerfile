FROM rust:1.90.0-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY migrations ./migrations
COPY static ./static
RUN cargo build --locked --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/pagerune /usr/local/bin/pagerune
COPY templates ./templates
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["pagerune"]
