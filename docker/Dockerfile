# Builds telelog-server from source:
#   docker build -f docker/Dockerfile -t telelog-server .
# Tagged releases publish ghcr.io/tolaniverse/telelog-server, built by docker/release.Dockerfile.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p telelog-server \
    && cp target/release/telelog-server /telelog-server

FROM gcr.io/distroless/cc-debian12
COPY --from=build /telelog-server /usr/local/bin/telelog-server
# Inside a container the server must listen on all interfaces; it still refuses to start
# without TELELOG_TOKEN unless --allow-unauthenticated is passed.
ENV TELELOG_LISTEN=0.0.0.0:7070
EXPOSE 7070
ENTRYPOINT ["/usr/local/bin/telelog-server"]
