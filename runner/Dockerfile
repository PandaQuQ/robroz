# Produces robroz-runner:local. Build from the repo root:
#   docker build -f runner/Dockerfile -t robroz-runner:local .
# CI compiles hive-runner with cargo and does not build this image.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p hive-runner

FROM debian:bookworm-slim
COPY --from=build /src/target/release/hive-runner /usr/local/bin/hive-runner
# The host passes --user {uid}:{gid}, so files in /workspace stay owned by the
# host user. The runner needs no home dir and writes only under /workspace, so
# any uid works. Default to nobody rather than root when --user is not given.
USER 65534:65534
ENTRYPOINT ["/usr/local/bin/hive-runner"]
