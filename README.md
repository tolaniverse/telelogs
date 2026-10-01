# Telelogs

[![CI](https://github.com/tolaniverse/telelogs/actions/workflows/ci.yml/badge.svg)](https://github.com/tolaniverse/telelogs/actions/workflows/ci.yml)

Open-source log aggregator: a native desktop app (built with [GPUI](https://gpui.rs)) that streams logs from your Docker, Kubernetes and VM environments, with long retention in your own object-storage buckets.

## Components

| Crate | Role |
|---|---|
| `telelog-core` | Shared log model (`LogRecord`, `Target`, `Filter`) |
| `telelog-proto` | gRPC API (`proto/telelog/v1/logs.proto`) and conversions |
| `telelog-sources` | Log sources. Docker today; Kubernetes and VMs next |
| `telelog-server` | Self-hostable server that collects logs and streams them to the app |
| `telelog-app` | GPUI desktop client (`telelogs` binary) |

## Running locally

Requires Rust and a running Docker daemon.

```sh
cargo run -p telelog-server            # listens on 127.0.0.1:7070
cargo run -p telelog-app               # connects to http://127.0.0.1:7070
```

Use `--listen` / `TELELOG_LISTEN` and `--server` / `TELELOG_SERVER` to change addresses.

## Securing the server

On `127.0.0.1` the server runs open, for local use. To reach it from other machines, give it a token
(and ideally TLS); it refuses to listen beyond localhost without one.

```sh
export TELELOG_TOKEN=$(telelog-server gen-token)
telelog-server --listen 0.0.0.0:7070 --tls-cert cert.pem --tls-key key.pem
```

Then point the app at it with the same token. For a self-signed certificate, pass the CA it was issued by:

```sh
TELELOG_TOKEN=... telelogs --server https://logs.example.com:7070 --ca-cert ca.pem
```

All options can also be set as environment variables: `TELELOG_LISTEN`, `TELELOG_TOKEN`, `TELELOG_TLS_CERT`,
`TELELOG_TLS_KEY`, `TELELOG_SERVER`, `TELELOG_CA_CERT`. If TLS is terminated by a proxy in front of the server,
the token still works over the proxy's TLS; `--allow-unauthenticated` exists only for proxies that do their own auth.

Try it with a noisy container:

```sh
docker run -d --rm --name telelog-demo alpine sh -c \
  'i=0; while true; do i=$((i+1)); echo "INFO request $i ok"; sleep 0.5; done'
```

## License

Telelogs is open source under two licenses:

| Component | License |
|---|---|
| `telelog-server` | [AGPL-3.0-only](LICENSE-AGPL) |
| `telelog-app`, `telelog-core`, `telelog-proto`, `telelog-sources` | [Apache-2.0](LICENSE-APACHE) |

You can self-host, modify and use the server freely. If you offer a modified server to others over a network, the AGPL requires you to publish your changes. The app and libraries are permissive, so you can embed them or build integrations without restriction.

Each crate's `Cargo.toml` declares its license.

### Third-party assets

The app bundles [Geist and Geist Mono](https://github.com/vercel/geist-font) (SIL Open Font License, `crates/telelog-app/assets/fonts/OFL.txt`) and [Phosphor icons](https://phosphoricons.com) (MIT, `crates/telelog-app/assets/icons/LICENSE-PHOSPHOR`).
