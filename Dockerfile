FROM node:24-bookworm-slim AS web
WORKDIR /web
COPY frontend/package*.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run check && npm run build

FROM rust:1.99.1-bookworm AS rust
WORKDIR /build
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
COPY backend/migrations ./migrations
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home subdisc && mkdir /data && chown subdisc:subdisc /data
WORKDIR /app
COPY --from=rust /build/target/release/subdisc /usr/local/bin/subdisc
COPY --from=web /web/dist /app/public
ENV STATIC_DIR=/app/public DATABASE_URL=sqlite:///data/subdisc.db BIND=0.0.0.0:3000
USER subdisc
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s CMD curl -fsS http://127.0.0.1:3000/api/health || exit 1
ENTRYPOINT ["subdisc"]
