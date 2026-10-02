# Image for tagged releases: wraps the telelog-server binaries the release workflow built
# (dist/telelog-server-linux-<arch>), so nothing is compiled under emulation.
FROM gcr.io/distroless/cc-debian12
ARG TARGETARCH
COPY dist/telelog-server-linux-${TARGETARCH} /usr/local/bin/telelog-server
# Inside a container the server must listen on all interfaces; it still refuses to start
# without TELELOG_TOKEN unless --allow-unauthenticated is passed.
ENV TELELOG_LISTEN=0.0.0.0:7070
EXPOSE 7070
ENTRYPOINT ["/usr/local/bin/telelog-server"]
