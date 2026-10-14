# @feature runtime
# @feature persistence
# @spec docs/features/runtime.md
# @spec docs/features/persistence.md
FROM rust:1.88-bookworm AS builder

RUN apt-get update \
    && apt-get install --yes --no-install-recommends libpq-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .
RUN cargo build --release --locked --bin nexus-sqlite-export \
    && cargo build --release --locked --no-default-features --features postgres --bin nexus-server

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl libpq5 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 nexus \
    && install --directory --owner nexus --group nexus /var/lib/nexus

COPY --from=builder /build/target/release/nexus-server /usr/local/bin/nexus-server
COPY --from=builder /build/target/release/nexus-sqlite-export /usr/local/bin/nexus-sqlite-export
COPY --chmod=0755 scripts/docker-healthcheck.sh /usr/local/bin/nexus-healthcheck
COPY --chmod=0755 scripts/docker-import-sqlite.sh /usr/local/bin/nexus-import-sqlite

USER nexus
ENV NEXUS_SECRET_FILE=/var/lib/nexus/service.secret
ENV NEXUS_HEALTH_URL=http://127.0.0.1:7337/readyz
ENV NEXUS_HEALTH_READY_FILE=/tmp/nexus-readiness-observed
ENV NEXUS_IMPORT_SQLITE_IF_PRESENT=/var/lib/nexus-import/nexus.db
VOLUME ["/var/lib/nexus"]
EXPOSE 7337
# The server writes the readiness marker only after initialization. A bounded pre-ready
# watchdog also terminates a hung connect or migration; consecutive later failures signal
# its handled PID 1 so the restart policy establishes a fresh connection.
HEALTHCHECK --interval=10s --timeout=5s --start-period=15s --retries=3 \
    CMD ["nexus-healthcheck"]

ENTRYPOINT ["nexus-server"]
CMD ["--listen", "0.0.0.0:7337"]
