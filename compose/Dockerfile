# syntax=docker/dockerfile:1

FROM --platform=$BUILDPLATFORM rust:1.97.1-slim-bookworm@sha256:2775a09d208ff0d7c1f50490c45b62db929e87ba1dcbc3f2132ac71a704bcdd3 AS api-builder
ARG TARGETARCH
ARG BUILDARCH
ARG CARGO_BUILD_JOBS
WORKDIR /build/apps/api
ENV SQLX_OFFLINE=true
RUN set -eux; \
    if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDARCH" != "arm64" ]; then \
        dpkg --add-architecture arm64; \
        apt-get update; \
        apt-get install -y --no-install-recommends \
            pkg-config libssl-dev libssl-dev:arm64 \
            gcc-aarch64-linux-gnu libc6-dev-arm64-cross; \
        rustup target add aarch64-unknown-linux-gnu; \
    else \
        apt-get update; \
        apt-get install -y --no-install-recommends pkg-config libssl-dev; \
    fi; \
    rm -rf /var/lib/apt/lists/*
COPY compose/build-api.sh /usr/local/bin/panda-build-api
COPY apps/api/Cargo.toml apps/api/Cargo.lock ./
RUN sh /usr/local/bin/panda-build-api seed
COPY apps/api/ ./
RUN sh /usr/local/bin/panda-build-api build

FROM --platform=$BUILDPLATFORM node:24.18.0-alpine@sha256:a0b9bf06e4e6193cf7a0f58816cc935ff8c2a908f81e6f1a95432d679c54fbfd AS web-builder
RUN corepack enable
WORKDIR /build
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/web/package.json apps/web/
COPY packages/config/package.json packages/config/
COPY packages/dashboards/package.json packages/dashboards/
COPY packages/hooks/package.json packages/hooks/
COPY packages/themes/package.json packages/themes/
COPY packages/types/package.json packages/types/
COPY packages/ui/package.json packages/ui/
COPY patches/image-size@2.0.2.patch patches/
RUN pnpm config set allowUnusedPatches true --location project \
    && pnpm install --frozen-lockfile
COPY apps/web/ apps/web/
COPY packages/config/ packages/config/
COPY packages/dashboards/ packages/dashboards/
COPY packages/hooks/ packages/hooks/
COPY packages/themes/ packages/themes/
COPY packages/types/ packages/types/
COPY packages/ui/ packages/ui/
RUN pnpm --filter @panda/web build

FROM postgres:18.4-bookworm@sha256:1961f96e6029a02c3812d7cb329a3b03a3ac2bb067058dec17b0f5596aca9296 AS runtime
ARG PANDA_BUILD_VERSION=unknown
ARG PANDA_RELEASE_SHA=unknown
LABEL org.opencontainers.image.source="https://github.com/your-org/r-the-drive"
ENV TZ=UTC XDG_CACHE_HOME=/data/cache PANDA_BUILD_VERSION=${PANDA_BUILD_VERSION}

RUN apt-get update && apt-get upgrade -y --no-install-recommends \
    && apt-get install -y --no-install-recommends \
    ca-certificates curl libpcre2-8-0 libssl3 nginx tini util-linux \
    && apt-mark manual postgresql-client-18 \
    && apt-get purge -y postgresql-18 postgresql-18-jit libllvm19 libxml2 libxslt1.1 gpg libsqlite3-0 \
    && apt-get autoremove -y --purge \
    && find / -xdev -type f -perm /6000 -exec chmod a-s '{}' + \
    && rm -rf /var/lib/apt/lists/* /etc/nginx/sites-enabled /usr/share/nginx/html \
        /usr/local/bin/gosu \
    && groupadd -r -g 1001 panda \
    && useradd -r -u 1001 -g 1001 panda \
    && mkdir -p /backups /data/cache \
    && chown -R 1001:1001 /backups /data/cache

WORKDIR /app
RUN printf '%s\n' "$PANDA_RELEASE_SHA" > /app/release-sha
COPY --chown=1001:1001 --from=api-builder /build/apps/api/panda-api ./panda-api
COPY --chown=1001:1001 --from=api-builder /build/apps/api/panda-restore-agent ./panda-restore-agent
COPY --chown=1001:1001 --from=api-builder /build/apps/api/rebaseline_db ./rebaseline_db
COPY --chown=1001:1001 --from=api-builder /build/apps/api/panda-keys ./panda-keys
COPY --chown=1001:1001 --from=api-builder /build/apps/api/migrations ./migrations
COPY --chown=1001:1001 --from=web-builder /build/apps/web/dist ./dist
COPY --chmod=0644 compose/nginx/nginx.conf /etc/nginx/nginx.conf
COPY --chmod=0644 compose/nginx/security-headers.conf /etc/nginx/security-headers.conf
COPY --chown=1001:1001 compose/production-services.sh compose/production-healthcheck.sh compose/render-origin-gate.sh ./
RUN sed -i 's/\r$//' /app/production-services.sh /app/production-healthcheck.sh /app/render-origin-gate.sh \
    && chmod 0755 /app/production-services.sh /app/production-healthcheck.sh /app/render-origin-gate.sh

EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD /app/production-healthcheck.sh

USER 1001:1001
ENTRYPOINT ["/usr/bin/tini", "--", "/app/production-services.sh"]
