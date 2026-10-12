#!/bin/sh
# Installs telelog-collector from the latest Telelogs release on GitHub, after checking it against
# the release's SHA256SUMS.
#
#   curl -fsSL https://raw.githubusercontent.com/tolaniverse/telelogs/main/install-collector.sh | sh
#
# Environment: TELELOG_VERSION=v0.4.0 picks a release; TELELOG_INSTALL_DIR picks where it goes
# (default /usr/local/bin when writable or with sudo, else ~/.local/bin).
set -eu

repo="tolaniverse/telelogs"
say() { printf '%s\n' "$*" >&2; }
fail() { say "telelog-collector install: $*"; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "needs $1"; }
need curl
need tar
need uname

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=darwin ;;
  *) fail "no build for $(uname -s); build from source with cargo instead" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=amd64 ;;
  aarch64 | arm64) arch=arm64 ;;
  *) fail "no build for $(uname -m); build from source with cargo instead" ;;
esac

tag="${TELELOG_VERSION:-}"
if [ -z "$tag" ]; then
  # The latest release page redirects to its tag.
  tag=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest" | sed 's|.*/tag/||')
  [ -n "$tag" ] || fail "couldn't find the latest release"
fi
version="${tag#v}"
name="telelog-collector-$version-$os-$arch"
base="https://github.com/$repo/releases/download/$tag"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
say "Downloading telelog-collector $version for $os-$arch…"
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz" || fail "no $name.tar.gz in release $tag"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" || fail "no SHA256SUMS in release $tag"

expected=$(grep " $name.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$expected" ] || fail "$name.tar.gz isn't listed in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)
else
  actual=$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d' ' -f1)
fi
[ "$expected" = "$actual" ] || fail "checksum mismatch for $name.tar.gz; not installing"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
bin="$tmp/$name/telelog-collector"
chmod +x "$bin"

dir="${TELELOG_INSTALL_DIR:-}"
sudo=""
if [ -z "$dir" ]; then
  if [ -w /usr/local/bin ]; then
    dir=/usr/local/bin
  elif command -v sudo >/dev/null 2>&1; then
    # sudo asks for the password on the terminal, even when this script is piped from curl.
    dir=/usr/local/bin
    sudo=sudo
  else
    dir="$HOME/.local/bin"
  fi
fi
$sudo mkdir -p "$dir"
$sudo install -m 0755 "$bin" "$dir/telelog-collector"
say "Installed $dir/telelog-collector ($("$dir/telelog-collector" --version))."
case ":$PATH:" in
  *":$dir:"*) ;;
  *) say "Add $dir to your PATH to run it by name." ;;
esac
say "Next: telelog-collector --enroll <token> --gateway <gateway>, both from your Telelog Cloud dashboard."
