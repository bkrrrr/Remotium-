# A dev container: toolchains, the Remotium agent and a pre-baked VSCodium REH.
# Other VSCodium versions are downloaded on demand into the /data volume.

FROM rust:1-bookworm AS agent
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p remotium-agent \
 && install -Dm755 target/release/remotium-agent /out/remotium-agent

FROM debian:trixie-slim
ARG VSCODIUM_VERSION=1.143.17232
ARG VSCODIUM_RELEASE=
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl git openssh-client procps tini less nodejs \
 && rm -rf /var/lib/apt/lists/*

# Pre-bake the pinned VSCodium REH, verified against the release's .sha256 file.
RUN set -eu; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in amd64) a=x64 ;; arm64) a=arm64 ;; armhf) a=armhf ;; *) echo "unsupported $arch"; exit 1 ;; esac; \
    v="${VSCODIUM_VERSION}${VSCODIUM_RELEASE:+.$VSCODIUM_RELEASE}"; \
    url="https://github.com/VSCodium/vscodium/releases/download/${v}/vscodium-reh-linux-${a}-${v}.tar.gz"; \
    curl -fsSL -o /tmp/reh.tgz "$url"; \
    echo "$(curl -fsSL "$url.sha256" | cut -d' ' -f1)  /tmp/reh.tgz" | sha256sum -c -; \
    mkdir -p /tmp/reh && tar -xzf /tmp/reh.tgz -C /tmp/reh; \
    commit="$(node -p 'require("/tmp/reh/product.json").commit')"; \
    mkdir -p /opt/vscodium-server && mv /tmp/reh "/opt/vscodium-server/$commit"; \
    rm /tmp/reh.tgz; \
    echo "pre-baked VSCodium REH $v ($commit)"

COPY --from=agent /out/remotium-agent /usr/local/bin/remotium-agent
COPY deploy/devenv/entrypoint.sh /usr/local/bin/devenv-entrypoint
COPY LICENSE THIRD_PARTY_NOTICES.md /usr/share/doc/remotium/
COPY deploy/devenv/workspace /workspace

RUN useradd -m -u 1000 -s /bin/bash dev \
 && mkdir -p /data \
 && chown -R dev:dev /workspace /data
USER dev
WORKDIR /workspace
ENV AGENT_DATA_DIR=/data \
    AGENT_WORKSPACE_ROOT=/workspace \
    AGENT_REH_SEARCH_PATH=/opt/vscodium-server
VOLUME ["/data"]
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/devenv-entrypoint"]
