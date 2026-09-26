#!/usr/bin/env python3
"""Builds each entry's module again from its public source, and compares it to the checksum.

This is the check a reviewer cannot do by reading: an entry names a binary, and nothing about the
binary says it came from the source beside it. So the source is fetched at the commit the entry
names, built in a container pinned to one Rust release, and the bytes that come out are hashed. If
that hash is not module.sha256, the entry is refused — whatever else it says is true.

Nothing the entry says is a command. The repository, the commit, the sub-directory and the file to
hash come from it; what is run on them is fixed here. The build itself still runs the submission's
code — build scripts and macros are code — so it runs in a throwaway container, as nobody, with no
network once the dependencies are down and nothing of the host mounted but the checkout.

    python3 scripts/verify_build.py                      # every entry
    python3 scripts/verify_build.py plugins/hello.json   # one
    python3 scripts/verify_build.py --write              # maintainers: take the rebuilt bytes
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import validate
from validate import Problem

ROOT = validate.ROOT
MODULES = ROOT / "modules"

# The compiler, by digest. A tag can be rebuilt and republished; a digest is the image or nothing,
# which is the whole point of doing this in a container.
IMAGE = "rust@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e"
IMAGE_TAG = f"rust:{validate.BUILD_TOOLCHAIN}-bookworm"
TARGET = "wasm32-unknown-unknown"

# Fixed inside the container, so every build sees the same paths and a path that leaks into the
# output leaks the same bytes on every machine.
WORK = "/src"
# The compiler and the dependencies, kept beside the checkout rather than in the image: the
# wasm32 target is added while there is still a network, and has to survive into the build, which
# runs in a second container with none.
TOOLS = "/build"
CARGO_HOME = f"{TOOLS}/cargo"
RUSTUP_HOME = f"{TOOLS}/rustup"

FETCH_MINUTES = 15
BUILD_MINUTES = 30


def run(command: list[str], timeout: int, **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(command, capture_output=True, text=True, timeout=timeout, **kwargs)


def tail(process: subprocess.CompletedProcess, lines: int = 12) -> str:
    output = (process.stderr or process.stdout or "").strip().splitlines()
    return "\n      ".join(output[-lines:]) or "no output"


def clone(entry: dict, into: Path) -> None:
    """The source, at the commit the entry names, fetched the way a stranger would fetch it."""
    build = entry["build"]
    repository, rev = build["repository"], build["rev"]
    env = {**os.environ, "GIT_TERMINAL_PROMPT": "0", "GIT_ASKPASS": "true", "GIT_CONFIG_GLOBAL": "/dev/null"}

    for command in (
        ["git", "init", "--quiet", str(into)],
        ["git", "-C", str(into), "remote", "add", "origin", repository],
        # The commit itself, not a branch: whatever the branch points at today is not what was
        # reviewed, and a shallow fetch of a sha only succeeds if that sha is really in there.
        ["git", "-C", str(into), "fetch", "--quiet", "--depth", "1", "origin", rev],
        ["git", "-C", str(into), "checkout", "--quiet", "FETCH_HEAD"],
    ):
        done = run(command, timeout=FETCH_MINUTES * 60, env=env)
        if done.returncode != 0:
            if "fetch" in command:
                raise Problem(f"commit {rev[:12]} is not in {repository}, or the repository is not public:\n      {tail(done)}")
            raise Problem(f"could not read {repository}:\n      {tail(done)}")

    at = run(["git", "-C", str(into), "rev-parse", "HEAD"], timeout=60)
    if at.stdout.strip() != rev:
        raise Problem(f"{repository} gave {at.stdout.strip()[:12]} for {rev[:12]}")


def docker(args: list[str], inside: str, checkout: Path, cargo: Path, minutes: int) -> subprocess.CompletedProcess:
    # Whatever the container writes ends up on this machine through the mount, so it is handed back
    # owned by whoever is running this and not by root.
    own = f"chown -R {os.getuid()}:{os.getgid()} {WORK} {TOOLS} 2>/dev/null || true"
    return run(
        [
            "docker", "run", "--rm", "--platform", "linux/amd64",
            # The submission's build scripts run here. They get the checkout and nothing else.
            "--volume", f"{checkout}:{WORK}",
            "--volume", f"{cargo}:{TOOLS}",
            "--workdir", WORK,
            "--env", f"CARGO_HOME={CARGO_HOME}",
            "--env", f"RUSTUP_HOME={RUSTUP_HOME}",
            # rustup reads rust-toolchain.toml out of the checkout otherwise, which would let a
            # submission choose its own compiler — and with it, its own bytes.
            "--env", f"RUSTUP_TOOLCHAIN={validate.BUILD_TOOLCHAIN}",
            "--env", "CARGO_TERM_COLOR=never",
            # Two identical builds an hour apart, not two builds an hour apart.
            "--env", "SOURCE_DATE_EPOCH=0",
            *args,
            IMAGE, "bash", "-euo", "pipefail", "-c", f"trap '{own}' EXIT\n{inside}",
        ],
        timeout=minutes * 60,
    )


# Named in a licence field but nowhere in the repository: worth saying, not worth refusing over —
# the file may sit somewhere this check does not look.
LICENCE_FILES = ("LICENSE", "LICENCE", "LICENSE.md", "LICENCE.md", "LICENSE.txt", "COPYING", "COPYING.md")


def manifest(entry: dict, checkout: Path) -> list[str]:
    """The plugin's own manifest, at the commit that was built, against the entry beside it.

    An entry is what Agentty shows; agentty-plugin.json is what the module actually is. If they
    disagree, the marketplace is advertising something the plugin is not — a version that was never
    built, or a permission the panel never declared.
    """
    plugin_path = checkout / entry["build"]["path"] / "agentty-plugin.json"
    if not plugin_path.is_file():
        raise Problem(f"{entry['build']['path']}/agentty-plugin.json is not in the repository: a plugin carries its own manifest")
    try:
        declared = json.loads(plugin_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        raise Problem(f"{entry['build']['path']}/agentty-plugin.json is not valid JSON: {err}") from err
    if not isinstance(declared, dict):
        raise Problem("agentty-plugin.json is a JSON object")

    for field in ("id", "version"):
        if str(declared.get(field, "")) != str(entry[field]):
            raise Problem(f"agentty-plugin.json says {field} {declared.get(field)!r}, the entry says {entry[field]!r}")
    if int(declared.get("apiVersion", 1)) != int(entry.get("apiVersion", 1)):
        raise Problem(f"agentty-plugin.json says apiVersion {declared.get('apiVersion', 1)}, the entry says {entry.get('apiVersion', 1)}")
    if sorted(declared.get("permissions", [])) != sorted(entry.get("permissions", [])):
        raise Problem(
            "the entry and agentty-plugin.json ask for different permissions.\n"
            f"      entry:     {sorted(entry.get('permissions', [])) or 'none'}\n"
            f"      manifest:  {sorted(declared.get('permissions', [])) or 'none'}"
        )

    notes = []
    if not any((checkout / name).is_file() for name in LICENCE_FILES):
        notes.append(f"the entry says {entry['license']}, but the repository carries no LICENSE file")
    return notes


def build(entry: dict, checkout: Path, cargo: Path) -> bytes:
    """The module, built from that checkout. Returns its bytes."""
    build_spec = entry["build"]
    path = build_spec["path"]
    artifact = Path(WORK) / path / build_spec["artifact"]

    if not (checkout / path).is_dir():
        raise Problem(f"build.path {path} is not in the repository at {build_spec['rev'][:12]}")
    if not (checkout / path / "Cargo.lock").is_file():
        raise Problem(f"{path}/Cargo.lock is not committed: without it the dependencies, and the module, can change under the entry")

    # Dependencies first, with the network, and only what Cargo.lock already pins. Then the build,
    # with no network at all — that is the part that runs the submission's own code.
    fetched = docker(
        [],
        f"mkdir -p {CARGO_HOME} {RUSTUP_HOME} && cp -a /usr/local/rustup/. {RUSTUP_HOME}/ "
        f"&& rustup target add {TARGET} >/dev/null && cd {path} && cargo fetch --locked --target {TARGET}",
        checkout, cargo, FETCH_MINUTES,
    )
    if fetched.returncode != 0:
        raise Problem(f"the dependencies in {path}/Cargo.lock could not be fetched:\n      {tail(fetched)}")

    built = docker(
        ["--network", "none"],
        f"cd {path} && {' '.join(validate.BUILD_COMMAND)}",
        checkout, cargo, BUILD_MINUTES,
    )
    if built.returncode != 0:
        raise Problem(f"{path} does not build with Rust {validate.BUILD_TOOLCHAIN}:\n      {tail(built)}")

    produced = checkout / path / build_spec["artifact"]
    if not produced.is_file():
        raise Problem(f"the build did not write {path}/{build_spec['artifact']} (looked at {artifact})")
    data = produced.read_bytes()
    if not data.startswith(b"\x00asm"):
        raise Problem(f"{build_spec['artifact']} is not a WebAssembly module")
    return data


def verify(path: Path, write: bool) -> None:
    entry = validate.check(path)
    if entry.get("official") and "build" not in entry:
        # Its source is the publisher's own and may be private; the module is the file merged here.
        print(f"  ok  {path.name}: official, served from modules/ (not rebuilt)")
        return
    with tempfile.TemporaryDirectory(prefix="agentty-build-") as folder:
        checkout, cargo = Path(folder) / "source", Path(folder) / "cargo"
        cargo.mkdir()
        clone(entry, checkout)
        notes = manifest(entry, checkout)
        data = build(entry, checkout, cargo)
        shutil.rmtree(folder, ignore_errors=True)

    for note in notes:
        print(f"note  {path.name}: {note}")
    digest, size = hashlib.sha256(data).hexdigest(), len(data)
    module = entry["module"]
    if digest == module["sha256"] and size == module["size"]:
        print(f"  ok  {path.name}: {digest[:16]}… from {entry['build']['rev'][:12]}")
        return

    if not write:
        raise Problem(
            f"the module does not come from the source it names.\n"
            f"      entry: sha256 {module['sha256']}, {module['size']} bytes\n"
            f"      built: sha256 {digest}, {size} bytes\n"
            f"      built from {entry['build']['repository']} at {entry['build']['rev'][:12]}, {entry['build']['path']}, Rust {validate.BUILD_TOOLCHAIN}"
        )

    module["sha256"], module["size"] = digest, size
    path.write_text(json.dumps(entry, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    served = MODULES / Path(module["url"]).name
    if served.parent == MODULES:
        served.write_bytes(data)
        served.chmod(0o644)
    print(f"  written  {path.name}: sha256 {digest}, {size} bytes")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("entries", nargs="*", type=Path, help="entries to verify (default: all of them)")
    parser.add_argument("--write", action="store_true",
                        help="take the rebuilt bytes as the truth: update the entry, and the module in modules/")
    args = parser.parse_args()

    if shutil.which("docker") is None:
        print("docker is needed: the module is built in a pinned container so the bytes do not depend on this machine.")
        return 2
    have = run(["docker", "image", "inspect", IMAGE], timeout=120)
    if have.returncode != 0:
        pulled = run(["docker", "pull", "--platform", "linux/amd64", IMAGE_TAG], timeout=20 * 60)
        if pulled.returncode != 0 or run(["docker", "image", "inspect", IMAGE], timeout=120).returncode != 0:
            print(f"could not get {IMAGE_TAG} as {IMAGE}:\n      {tail(pulled)}")
            return 2

    failed = 0
    for path in (args.entries or validate.entries()):
        try:
            verify(path.resolve(), args.write)
        except Problem as problem:
            failed += 1
            print(f"FAIL  {path.name}: {problem}")
        except subprocess.TimeoutExpired:
            failed += 1
            print(f"FAIL  {path.name}: the build did not finish in time")
        except OSError as err:
            failed += 1
            print(f"FAIL  {path.name}: {err}")

    if failed:
        print(f"\n{failed} module(s) do not match the source they name.")
        return 1
    print("\nEvery module was built again from its source and came out the same.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
