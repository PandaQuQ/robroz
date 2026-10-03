# Produces openbots-runner:local. Build from the repo root:
#   docker build -f runner/Dockerfile -t openbots-runner:local .
# CI compiles hive-runner with cargo and does not build this image.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p hive-runner

FROM debian:bookworm-slim
COPY --from=build /src/target/release/hive-runner /usr/local/bin/hive-runner
ENTRYPOINT ["/usr/local/bin/hive-runner"]
