#!/usr/bin/env bash
# Builds every plugin in src/ and puts its module in modules/<id>-<version>.wasm — the file an
# entry points at. Prints the checksum and size each entry needs.
set -euo pipefail
cd "$(dirname "$0")/.."

rustup target add wasm32-unknown-unknown >/dev/null
mkdir -p modules

for dir in src/*/; do
  id="$(basename "$dir")"
  version="$(python3 -c "import json,sys;print(json.load(open('$dir/agentty-plugin.json'))['version'])")"
  crate="$(python3 -c "
import re,sys
print(re.search(r'name\s*=\s*\"([^\"]+)\"', open('$dir/Cargo.toml').read()).group(1).replace('-','_'))")"
  (cd "$dir" && cargo build --release --target wasm32-unknown-unknown)
  built="$dir/target/wasm32-unknown-unknown/release/$crate.wasm"
  out="modules/$id-$version.wasm"
  # A version that is already published keeps its bytes: an entry points at what was reviewed,
  # and a wasm build is not reproducible across machines or SDK versions. Raise the version.
  if [ -f "$out" ] && ! cmp -s "$built" "$out"; then
    printf '%s is already here and differs - raise the version in %s\n' "$out" "$dir/agentty-plugin.json" >&2
    continue
  fi
  cp "$built" "$out"
  chmod 644 "$out"
  printf '%s\n  sha256 %s\n  size   %s\n' "$out" "$(shasum -a 256 "$out" | cut -d' ' -f1)" "$(wc -c < "$out" | tr -d ' ')"
done
