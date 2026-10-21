# Two images from one build: `polarion-cli` (target `cli`) and `polarion-mcp` (target `mcp`, the
# default, last stage).
#
#   docker build --target cli -t polarion-cli .
#   docker run --rm -e POLARION_URL=... -e POLARION_TOKEN=... polarion-cli items get MP-42
#
#   docker build -t polarion-mcp .
#   docker run --rm -p 127.0.0.1:8787:8787 \
#     -e POLARION_URL=https://polarion.example.com/polarion \
#     -e POLARION_TOKEN=... \
#     -e POLARION_PROJECT=MYPROJECT \
#     -e POLARION_WRITE_PROJECTS=MYPROJECT,OTHER \
#     -e POLARION_MCP_AUTH_TOKEN=<16+ characters> \
#     polarion-mcp
#
# Stages: `build` (both release binaries), `test` (the workspace's tests; the images depend on it,
# so a failing test fails the build), `runtime` (slim Debian, non-root user, the entry point),
# `cli` and `mcp`. `--target build` or `--target test` stops early.
#
# Scenarios: POLARION_PROFILE selects one of the profiles built in (docker/profiles/<image>/,
# copied to /etc/polarion/profiles; see docker/entrypoint.sh). polarion-mcp: `full-access` (the
# default), `every-tool`, `no-deletes`, `read-only`; its tool selections are in
# /etc/polarion/mcp/*.toml (examples/mcp/config). polarion-cli: none by default (the environment
# as given), `read-only`, `full-access`. A profile only sets what the container was not given,
# except that `read-only` removes the write scope.

ARG RUST_VERSION=1
ARG DEBIAN_RELEASE=bookworm
# Where the official base images come from; a mirror avoids Docker Hub's rate limits, e.g.
# --build-arg BASE_REGISTRY=mirror.gcr.io/library.
ARG BASE_REGISTRY=docker.io/library
# The base images themselves, built from the arguments above unless given whole, e.g.
# --build-arg RUST_IMAGE=registry.example.com/rust:1-bookworm (the runtime image should be the
# same Debian release: the binaries link its glibc).
ARG RUST_IMAGE=${BASE_REGISTRY}/rust:${RUST_VERSION}-${DEBIAN_RELEASE}
ARG RUNTIME_IMAGE=${BASE_REGISTRY}/debian:${DEBIAN_RELEASE}-slim

FROM ${RUST_IMAGE} AS build

USER root

# Mermaid diagrams are measured with a system font (polarion-markdown's `render_svg`).
RUN apt-get update \
    && apt-get install -y --no-install-recommends fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --locked --profile dist -p polarion-cli -p polarion-mcp \
    && cp target/dist/polarion target/dist/polarion-mcp /usr/local/bin/

FROM build AS test
# The CI checks of CLAUDE.md that do not need rustfmt/clippy; nothing here contacts a server.
RUN cargo test --locked --workspace \
    && cargo test --locked -p polarion-markdown --no-default-features \
    && cargo check --locked -p polarion \
    && touch /tests-passed

FROM ${RUNTIME_IMAGE} AS runtime

USER root

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /home/polarion --uid 10001 polarion
# Only built after the tests passed.
COPY --from=test /tests-passed /usr/share/polarion/tests-passed
COPY --chmod=755 docker/entrypoint.sh /usr/local/bin/polarion-entrypoint
ENV XDG_CACHE_HOME=/home/polarion/.cache
USER polarion

FROM runtime AS cli
COPY --from=build /usr/local/bin/polarion /usr/local/bin/polarion
COPY docker/profiles/cli/ /etc/polarion/profiles/
ENV POLARION_PROFILE=
# Files named in arguments (`@desc.md`, `--out`) are relative to /work: mount the host's folder
# there, e.g. -v "$PWD:/work" (with --user "$(id -u):$(id -g)" to write files you own).
WORKDIR /work
ENTRYPOINT ["polarion-entrypoint", "polarion"]
CMD ["--help"]

FROM runtime AS mcp
COPY --from=build /usr/local/bin/polarion-mcp /usr/local/bin/polarion-mcp
COPY docker/profiles/mcp/ /etc/polarion/profiles/
COPY examples/mcp/config/ /etc/polarion/mcp/
ENV POLARION_PROFILE=full-access \
    POLARION_MCP_HTTP=0.0.0.0:8787
WORKDIR /home/polarion
EXPOSE 8787
ENTRYPOINT ["polarion-entrypoint", "polarion-mcp"]
