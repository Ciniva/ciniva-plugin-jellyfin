# Desktop apps (Jellyfin Desktop / Media Player) load the web client from the server, so ship the official one.
FROM jellyfin/jellyfin:10.11.11@sha256:aefb67e6a7ff1debdd154a78a7bbb780fd0c873d8639210a7f6a2016ad2b35db AS web

FROM rust:1-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release && rm -rf src
COPY src ./src
RUN touch src/main.rs && cargo build --release

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
ARG UID=1000
ARG GID=1000
RUN groupadd --gid ${GID} ciniva \
 && useradd --uid ${UID} --gid ${GID} --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin ciniva \
 && { find / -xdev -perm /6000 -type f -exec chmod a-s {} + 2>/dev/null || true; }
COPY --from=build /src/target/release/ciniva-jellyfin /usr/local/bin/ciniva-jellyfin
COPY --from=web /jellyfin/jellyfin-web /usr/share/jellyfin-web
ENV PORT=8096 \
    SERVER_NAME=Ciniva \
    WEB_DIR=/usr/share/jellyfin-web
USER ciniva:ciniva
EXPOSE 8096
ENTRYPOINT ["/usr/local/bin/ciniva-jellyfin"]
