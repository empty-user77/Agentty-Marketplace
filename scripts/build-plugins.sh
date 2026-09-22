#!/usr/bin/env bash
# Builds every plugin in src/ and puts its module in modules/<id>-<version>.wasm — the file an
# entry points at. Prints the checksum and size each entry needs.
#
# The build runs in the same pinned container CI uses, not with whatever rustc is on this machine,
# because the entry's checksum has to be the one CI arrives at independently. That is the whole
# point of the check: two people building the same commit get the same bytes, or the plugin does
# not go in the list.
set -euo pipefail
cd "$(dirname "$0")/.."

TOOLCHAIN="$(python3 -c "import sys;sys.path.insert(0,'scripts');import validate;print(validate.BUILD_TOOLCHAIN)")"
IMAGE="$(python3 -c "import sys;sys.path.insert(0,'scripts');import verify_build;print(verify_build.IMAGE)")"
TARGET=wasm32-unknown-unknown

if ! command -v docker >/dev/null; then
  echo "docker is needed: the modules are built in a pinned container so the bytes do not depend on this machine." >&2
  exit 2
fi
docker image inspect "$IMAGE" >/dev/null 2>&1 || docker pull --platform linux/amd64 "rust:$TOOLCHAIN-bookworm"

tools="$(mktemp -d)"
trap 'rm -rf "$tools"' EXIT
mkdir -p modules

in_container() {
  local network=$1 script=$2
  docker run --rm --platform linux/amd64 \
    --volume "$PWD:/src" --volume "$tools:/build" \
    --workdir /src \
    --env CARGO_HOME=/build/cargo --env RUSTUP_HOME=/build/rustup \
    --env "RUSTUP_TOOLCHAIN=$TOOLCHAIN" --env CARGO_TERM_COLOR=never --env SOURCE_DATE_EPOCH=0 \
    $network "$IMAGE" bash -euo pipefail -c \
    "trap 'chown -R $(id -u):$(id -g) /src /build 2>/dev/null || true' EXIT
     $script"
}

in_container "" "mkdir -p /build/cargo /build/rustup && cp -a /usr/local/rustup/. /build/rustup/ \
  && rustup target add $TARGET >/dev/null"

for dir in src/*/; do
  id="$(basename "$dir")"
  version="$(python3 -c "import json;print(json.load(open('$dir/agentty-plugin.json'))['version'])")"
  crate="$(python3 -c "
import re
print(re.search(r'name\s*=\s*\"([^\"]+)\"', open('$dir/Cargo.toml').read()).group(1).replace('-','_'))")"

  # Dependencies with the network, the build itself without: a build script is code, and it has no
  # business reaching anywhere once Cargo.lock has been honoured.
  in_container "" "cd $dir && cargo fetch --locked --target $TARGET"
  in_container "--network none" "cd $dir && cargo build --release --locked --offline --target $TARGET"

  built="$dir/target/$TARGET/release/$crate.wasm"
  out="modules/$id-$version.wasm"
  # A version that is already published keeps its bytes: an entry points at what was reviewed, and
  # everyone who installed it checked that checksum. Raise the version instead.
  if [ -f "$out" ] && ! cmp -s "$built" "$out"; then
    printf '%s is already here and differs - raise the version in %s\n' "$out" "$dir/agentty-plugin.json" >&2
    continue
  fi
  cp "$built" "$out"
  # A module is data, not a program to run: cargo's output is executable, the committed file is not.
  chmod 644 "$out"
  printf '%s\n  sha256 %s\n  size   %s\n' "$out" "$(shasum -a 256 "$out" | cut -d' ' -f1)" "$(wc -c < "$out" | tr -d ' ')"
done

cat <<'NEXT'

Next: put the version, sha256 and size in plugins/<id>.json, push the source, and set build.rev to
the commit you pushed. Then `python3 scripts/verify_build.py` builds it again from that commit and
checks it comes out the same — which is exactly what CI will do.
NEXT
