# Produces the robroz-host image. Build from the repo root:
#   docker build -f host/Dockerfile -t robroz-host:local .
# See docs/docker-host.md for how to run it (same-path data mount, docker.sock,
# token file). The image holds the host binary and the Docker CLI only.
# It has no Docker daemon, no Node and no npx.
FROM rust:1-bookworm AS build
# `.git` is not in the build context, so the commit for `--version` comes in
# as a build arg (CI passes the full sha; build.rs keeps 7 characters).
ARG ROBROZ_GIT_SHA=unknown
WORKDIR /src
COPY . .
# `[profile.release] strip = true` in the workspace Cargo.toml strips it.
RUN cargo build --release -p robroz-host \
    && /src/target/release/robroz-host --version

# A static Docker CLI. The host shells out to `docker run` / `inspect` / `rm`.
FROM docker:27.5.1-cli AS dockercli

FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="robroz-host" \
      org.opencontainers.image.description="robroz host: HTTP API, SQLite and the pump that starts runner containers through the Docker CLI" \
      org.opencontainers.image.source="https://github.com/PandaQuQ/robroz" \
      org.opencontainers.image.licenses="AGPL-3.0-only"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --user-group --create-home --shell /usr/sbin/nologin robroz
COPY --from=dockercli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=build /src/target/release/robroz-host /usr/local/bin/robroz-host

# Inside the container the host must listen on all interfaces, so a token is
# mandatory: without ROBROZ_TOKEN or ROBROZ_TOKEN_FILE the host exits with code 2.
# All three data paths sit under one root, and control is a sibling of groups.
# The compose example mounts the same absolute path from the engine host, because
# runner bind mounts are resolved by the engine, not by this container.
ENV ROBROZ_ADDR=0.0.0.0:8790 \
    ROBROZ_DB=/srv/robroz/robroz.db \
    ROBROZ_VOLUMES=/srv/robroz/groups \
    ROBROZ_CONTROL=/srv/robroz/control

# Non-root. To use the engine socket, add the host's docker group id with
# `group_add` (compose) or `--group-add` (docker run).
USER robroz
WORKDIR /srv/robroz
EXPOSE 8790
# 401 or 200 on GET /api/bros means the process is up (a token is required).
# The binary itself probes 127.0.0.1, so the image needs no curl.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD robroz-host --health
ENTRYPOINT ["/usr/local/bin/robroz-host"]
