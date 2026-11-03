# syntax=docker/dockerfile:1

# --- build -------------------------------------------------------------------------------------
FROM rust:bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked \
    -p asura-webgate -p asura-auth -p asura-world -p asura-supervisor

# --- runtime -----------------------------------------------------------------------------------
FROM debian:bookworm-slim

# psql: the supervisor's `schema` and `seed` commands apply migrations and seed rows through it.
RUN apt-get update \
 && apt-get install -y --no-install-recommends postgresql-client ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# The layout is the repository's own, on purpose: binaries at target/debug/<name>, the repository
# root at /src. The supervisor resolves config, tools and the migration directory from the path it
# was built with, and runs the services with the repository root as their working directory - so
# config/services.toml works unchanged inside the container. The image IS the machine.
WORKDIR /src
COPY --from=build /src/target/release/asura-webgate      /src/target/debug/asura-webgate
COPY --from=build /src/target/release/asura-auth        /src/target/debug/asura-auth
COPY --from=build /src/target/release/asura-world       /src/target/debug/asura-world
COPY --from=build /src/target/release/asura-supervisor  /src/target/debug/asura-supervisor
COPY config ./config
COPY tools ./tools
COPY crates/asura-db/migrations ./crates/asura-db/migrations
COPY data ./data
RUN mkdir -p logs

# docker stop sends SIGTERM; the supervisor treats it as the same quit as Ctrl+C, so the services
# are stopped and the registry rows marked `stopped` before the container exits.
#
# One string on purpose: with the `sh -c` entrypoint the whole chain must arrive as a single
# argument. config/services.docker.toml is the same service list as the host's, with the two
# client-facing services bound to 0.0.0.0 because published ports forward to the container's
# network address, not its loopback.
ENTRYPOINT ["/bin/sh", "-c"]
CMD ["target/debug/asura-supervisor schema && exec target/debug/asura-supervisor start --config config/services.docker.toml"]
