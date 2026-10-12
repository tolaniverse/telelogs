# Builds telelog-collector from source:
#   docker build -f docker/collector.Dockerfile -t telelog-collector .
# Tagged releases publish ghcr.io/tolaniverse/telelog-collector, built by collector.release.Dockerfile.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p telelog-collector \
    && cp target/release/telelog-collector /telelog-collector

FROM gcr.io/distroless/cc-debian12
COPY --from=build /telelog-collector /usr/local/bin/telelog-collector
ENV TELELOG_COLLECTOR_STATE=/state/collector.json
VOLUME /state
ENTRYPOINT ["/usr/local/bin/telelog-collector"]
