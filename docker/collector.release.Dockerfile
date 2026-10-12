# Image for tagged releases: wraps the telelog-collector binaries the release workflow built
# (dist/telelog-collector-linux-<arch>), so nothing is compiled under emulation.
FROM gcr.io/distroless/cc-debian12
ARG TARGETARCH
COPY dist/telelog-collector-linux-${TARGETARCH} /usr/local/bin/telelog-collector
# Its id and key live here; mount a volume on /state so a restarted container stays the same
# collector. With Docker it reads /var/run/docker.sock, which needs root (the image's default user).
ENV TELELOG_COLLECTOR_STATE=/state/collector.json
VOLUME /state
ENTRYPOINT ["/usr/local/bin/telelog-collector"]
