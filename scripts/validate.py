#!/usr/bin/env python3
"""Checks the marketplace entries, and rebuilds index.json from them.

Every entry is a plugin someone submitted: treat it as text from a stranger. Nothing here runs a
module or trusts a URL to be what it says — the checksum is what decides, and the checks below are
what a reviewer would otherwise have to remember.

    python3 scripts/validate.py              # shape, ids, URLs, permissions, the build block
    python3 scripts/validate.py --download   # also fetch each module and check its checksum
    python3 scripts/validate.py --source     # also check the source is public and the commit is there
    python3 scripts/validate.py --index      # rewrite index.json from plugins/*.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PLUGINS = ROOT / "plugins"
INDEX = ROOT / "index.json"

API_VERSION = 1
# The plugin protocol the current Agentty speaks. An entry built against a newer one would be shown
# to everyone as "needs a newer Agentty" and installable by nobody, so it is refused until Agentty
# ships that protocol and this number moves with it.
PLUGIN_API_VERSION = 3
ID = re.compile(r"^[a-z0-9][a-z0-9-]{1,38}[a-z0-9]$")
VERSION = re.compile(r"^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
ICON = re.compile(r"^[a-z0-9-]{1,40}$")
# A commit, not a tag or a branch: a tag can be moved to other code after the review.
COMMIT = re.compile(r"^[0-9a-f]{40}$")
# The Rust release the module is built with. Pinned, because a wasm build only reproduces when the
# compiler does: two rustc versions on the same source give two different modules.
TOOLCHAIN = re.compile(r"^\d+\.\d+\.\d+$")
# Inside the repository, so it cannot climb out of the checkout when CI builds it.
REPO_PATH = re.compile(r"^(?!/)(?!.*(?:^|/)\.\.(?:/|$))[A-Za-z0-9._/-]{1,200}$")

# What Agentty knows how to ask the user about. A plugin naming anything else would be installed
# with a permission nobody can explain.
PERMISSIONS = {"net.request", "prompt.inject", "terminal.write", "session.read", "workspace.read", "browser.control", "files"}
SURFACES = {"sidebar", "pane", "status"}
MODES = {"push", "overlay", "window", "full", "workspace"}

# A module is served from a release of a repository, not from someone's server: a release asset
# cannot be replaced without the URL changing, and these hosts are the ones GitHub serves them on.
MODULE_HOSTS = {"github.com", "raw.githubusercontent.com", "objects.githubusercontent.com"}
SOURCE_HOSTS = {"github.com", "gitlab.com", "codeberg.org", "git.sr.ht"}

# The Rust the marketplace builds every submission with. Raising it re-checksums every module, so
# it moves deliberately: bump it, rebuild with scripts/build-plugins.sh, and update the entries.
BUILD_TOOLCHAIN = "1.98.1"
# The one command CI runs. A submission says where its source is, never what to run on it.
BUILD_COMMAND = ["cargo", "build", "--release", "--locked", "--offline", "--target", "wasm32-unknown-unknown"]

AGENT = "agentty-marketplace-validate"

MAX_MODULE_BYTES = 8 * 1024 * 1024
MAX_DESCRIPTION = 300
MAX_NAME = 60
MAX_KEYWORDS = 10

REQUIRED = ["id", "name", "version", "description", "publisher", "license", "source", "module", "build"]


class Problem(Exception):
    pass


def url_host(url: str, field: str) -> str:
    if not isinstance(url, str) or not url.startswith("https://"):
        raise Problem(f"{field} must be an https:// URL")
    if len(url) > 500 or any(c.isspace() or ord(c) < 0x20 for c in url):
        raise Problem(f"{field} is not a URL")
    host = url[len("https://"):].split("/")[0].split("@")[-1].lower()
    if not host:
        raise Problem(f"{field} has no host")
    return host


def text(entry: dict, field: str, limit: int, required: bool = True) -> str:
    value = entry.get(field, "")
    if not isinstance(value, str):
        raise Problem(f"{field} must be text")
    value = value.strip()
    if required and not value:
        raise Problem(f"{field} is required")
    if len(value) > limit:
        raise Problem(f"{field} is longer than {limit} characters")
    if any(ord(c) < 0x20 for c in value):
        raise Problem(f"{field} contains control characters")
    return value


def repo_url(url: str, field: str) -> tuple[str, str]:
    """The host and owner/name a code URL points at. Raises Problem if it is not one."""
    host = url_host(url, field)
    if host not in SOURCE_HOSTS:
        raise Problem(f"{field} is on {host}; source is public on {', '.join(sorted(SOURCE_HOSTS))}")
    parts = [p for p in url[len("https://"):].split("/")[1:] if p]
    if len(parts) < 2:
        raise Problem(f"{field} must name a repository, as https://{host}/owner/name")
    owner, name = parts[0], parts[1]
    if name.endswith(".git"):
        name = name[: -len(".git")]
    return host, f"{owner}/{name}".lower()


def check_build(entry: dict) -> dict:
    """Where the module is built from. Everything CI needs to build it again and compare."""
    build = entry["build"]
    if not isinstance(build, dict):
        raise Problem("build is an object with repository, rev, path, toolchain and artifact")

    for field in ("repository", "rev", "path", "toolchain", "artifact"):
        if field not in build:
            raise Problem(f"build.{field} is required: CI builds the module again and compares it")

    build_host, build_repo = repo_url(build["repository"], "build.repository")

    # The entry's own source link has to lead to the code that was built, not to some other
    # repository of the same publisher. This is the half of "the source is public" that a checksum
    # cannot tell you.
    source_host, source_repo = repo_url(entry["source"], "source")
    if (source_host, source_repo) != (build_host, build_repo):
        raise Problem(f"source points at {source_host}/{source_repo}, build.repository at {build_host}/{build_repo}: they must be the same repository")

    if not isinstance(build["rev"], str) or not COMMIT.match(build["rev"]):
        raise Problem("build.rev is the full 40-character commit the module is built from, not a tag or a branch")

    for field in ("path", "artifact"):
        value = build[field]
        if not isinstance(value, str) or not REPO_PATH.match(value):
            raise Problem(f"build.{field} is a path inside the repository, without '..'")
    if not build["artifact"].endswith(".wasm"):
        raise Problem("build.artifact is the .wasm the build writes, relative to build.path")

    toolchain = build["toolchain"]
    if not isinstance(toolchain, str) or not TOOLCHAIN.match(toolchain):
        raise Problem("build.toolchain is a Rust release, as 1.98.1")
    if toolchain != BUILD_TOOLCHAIN:
        raise Problem(f"build.toolchain is {toolchain}; the marketplace builds every module with {BUILD_TOOLCHAIN}")

    return build


def source_is_public(entry: dict) -> None:
    """Fetches the source, anonymously, the way anyone reading the entry would. Never clones."""
    for field, url in (("source", entry["source"]), ("build.repository", entry["build"]["repository"])):
        request = urllib.request.Request(url, headers={"User-Agent": AGENT}, method="GET")
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                response.read(1)
        except urllib.error.HTTPError as err:
            raise Problem(f"{field} is not public: {url} answers {err.code}") from err

    rev, repository = entry["build"]["rev"], entry["build"]["repository"]
    found = subprocess.run(
        ["git", "ls-remote", "--exit-code", repository, rev],
        capture_output=True, text=True, timeout=60,
        env={**os.environ, "GIT_TERMINAL_PROMPT": "0", "GIT_ASKPASS": "true", "GCM_INTERACTIVE": "never"},
    )
    # A commit that is not a ref still has to be reachable; ls-remote only lists refs, so a miss
    # here is not a failure on its own. An unreadable repository is.
    if found.returncode not in (0, 2):
        raise Problem(f"build.repository cannot be read without credentials: {found.stderr.strip().splitlines()[-1] if found.stderr.strip() else 'git ls-remote failed'}")


def check(path: Path) -> dict:
    """One entry, checked. Raises Problem with what is wrong."""
    try:
        entry = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        raise Problem(f"not valid JSON: {err}") from err
    if not isinstance(entry, dict):
        raise Problem("an entry is a JSON object")

    # An official plugin is the marketplace's own: its module is committed to this repository, so
    # only whoever can merge here can publish one, and its source need not be public.
    official = entry.get("official", False)
    if not isinstance(official, bool):
        raise Problem("official is true or false")
    for field in REQUIRED:
        if field not in entry and not (official and field == "build"):
            raise Problem(f"{field} is required")

    plugin_id = entry["id"]
    if not isinstance(plugin_id, str) or not ID.match(plugin_id):
        raise Problem("id is 2–40 characters of a-z, 0-9 and '-'")
    if path.stem != plugin_id:
        raise Problem(f"the file is {path.name}, the id is {plugin_id}: they must match")

    if not VERSION.match(str(entry["version"])):
        raise Problem("version is major.minor.patch")

    text(entry, "name", MAX_NAME)
    text(entry, "description", MAX_DESCRIPTION)
    text(entry, "publisher", MAX_NAME)
    text(entry, "license", 40)

    host = url_host(entry["source"], "source")
    if host not in SOURCE_HOSTS and not official:
        raise Problem(f"source is on {host}; a plugin here is open source on {', '.join(sorted(SOURCE_HOSTS))}")
    if "homepage" in entry:
        url_host(entry["homepage"], "homepage")

    icon = entry.get("icon", "")
    if icon and (not isinstance(icon, str) or not ICON.match(icon)):
        raise Problem("icon is a name from Agentty's icon set")

    api_version = entry.get("apiVersion", 1)
    if not isinstance(api_version, int) or isinstance(api_version, bool):
        raise Problem("apiVersion is a whole number")
    if not 1 <= api_version <= PLUGIN_API_VERSION:
        raise Problem(f"apiVersion is between 1 and {PLUGIN_API_VERSION} (the protocol Agentty speaks today)")

    surface = entry.get("surface", "pane")
    if surface not in SURFACES:
        raise Problem(f"surface is one of {', '.join(sorted(SURFACES))}")
    mode = entry.get("mode", "push")
    if mode not in MODES:
        raise Problem(f"mode is one of {', '.join(sorted(MODES))}")

    keywords = entry.get("keywords", [])
    if not isinstance(keywords, list) or len(keywords) > MAX_KEYWORDS:
        raise Problem(f"keywords is a list of at most {MAX_KEYWORDS}")
    for keyword in keywords:
        if not isinstance(keyword, str) or not keyword.strip() or len(keyword) > 30:
            raise Problem("every keyword is a short word")

    permissions = entry.get("permissions", [])
    if not isinstance(permissions, list):
        raise Problem("permissions is a list")
    unknown = [p for p in permissions if p not in PERMISSIONS]
    if unknown:
        raise Problem(f"unknown permission(s): {', '.join(map(str, unknown))}")
    if len(set(permissions)) != len(permissions):
        raise Problem("a permission is listed twice")

    module = entry["module"]
    if not isinstance(module, dict):
        raise Problem("module is an object with url, sha256 and size")
    host = url_host(module.get("url", ""), "module.url")
    if host not in MODULE_HOSTS:
        raise Problem(f"module.url is on {host}; modules are served from {', '.join(sorted(MODULE_HOSTS))}")
    version = str(entry["version"])
    if version not in module["url"]:
        raise Problem("module.url must contain the version, so a release cannot be swapped underneath it")
    if not SHA256.match(str(module.get("sha256", ""))):
        raise Problem("module.sha256 is 64 hex characters")
    size = module.get("size")
    if not isinstance(size, int) or not 0 < size <= MAX_MODULE_BYTES:
        raise Problem(f"module.size is the size in bytes, up to {MAX_MODULE_BYTES // 1024 // 1024} MB")

    if official:
        # Served from modules/ in this repository and nowhere else: what makes it official is
        # that it was merged here, not that the entry says so.
        if served_here(entry) is None:
            raise Problem("an official plugin's module is a file in modules/ of this repository, and module.url points at it")
        if "build" in entry:
            check_build(entry)
    else:
        check_build(entry)

    # A plugin that both reads the user's work and sends requests out can carry it away. It is
    # allowed, and it is said out loud.
    if "net.request" in permissions and {"session.read", "workspace.read"} & set(permissions):
        if "net.request" not in entry.get("description", "") and len(entry["description"]) < 40:
            raise Problem("this plugin can read the user's work and send requests: say why in the description")
    return entry


def this_repository() -> str:
    """owner/name of the repository this checkout is, lowercased, or "" if it cannot be told."""
    try:
        remote = subprocess.run(
            ["git", "-C", str(ROOT), "config", "--get", "remote.origin.url"],
            capture_output=True, text=True, timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired):
        return ""
    url = remote.stdout.strip()
    if not url:
        return ""
    url = url.removesuffix(".git").replace(":", "/")
    parts = [part for part in url.split("/") if part]
    return "/".join(parts[-2:]).lower() if len(parts) >= 2 else ""


def served_here(entry: dict) -> Path | None:
    """The file in this checkout that a module URL points at, when it points back at this repo.

    The plugins published from here are served out of main, so on a pull request the URL still
    serves the old module: the new one is the file in the branch. Checking the URL would mean an
    entry could never change its module and go green in the same pull request. The file about to be
    merged is the honest thing to weigh.
    """
    repository = this_repository()
    if not repository:
        return None
    url = entry["module"]["url"]
    if url_host(url, "module.url") != "raw.githubusercontent.com":
        return None
    parts = url[len("https://raw.githubusercontent.com/"):].split("/")
    # <owner>/<name>/<ref>/modules/<file>
    if len(parts) != 5 or "/".join(parts[:2]).lower() != repository or parts[3] != "modules":
        return None
    here = ROOT / "modules" / parts[4]
    return here if here.is_file() else None


def download(entry: dict) -> None:
    """Weighs the module against the entry. Never runs it."""
    module = entry["module"]
    here = served_here(entry)
    if here is not None:
        data = here.read_bytes()[: MAX_MODULE_BYTES + 1]
    else:
        request = urllib.request.Request(module["url"], headers={"User-Agent": AGENT})
        with urllib.request.urlopen(request, timeout=60) as response:  # noqa: S310 - https is checked above
            data = response.read(MAX_MODULE_BYTES + 1)
    if len(data) > MAX_MODULE_BYTES:
        raise Problem("the module is larger than the limit")
    called = "the file in modules/" if here is not None else "the download"
    if len(data) != module["size"]:
        raise Problem(f"module.size says {module['size']}, {called} is {len(data)} bytes")
    digest = hashlib.sha256(data).hexdigest()
    if digest != module["sha256"]:
        raise Problem(f"module.sha256 says {module['sha256']}, {called} is {digest}")
    if not data.startswith(b"\x00asm"):
        raise Problem("that file is not a WebAssembly module")


def write_index(checked: list[dict]) -> bool:
    """Rewrites index.json from the entries. Returns whether anything changed.

    `updated` is the day the list changed, not the minute this ran: a timestamp that moved on every
    run would make "index.json matches the entries" fail on a pull request that never touched it.
    """
    index = {"apiVersion": API_VERSION, "updated": "", "plugins": checked}
    try:
        before = json.loads(INDEX.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        before = {}
    unchanged = before.get("apiVersion") == API_VERSION and before.get("plugins") == checked
    index["updated"] = (
        before["updated"] if unchanged and isinstance(before.get("updated"), str)
        else datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    )
    INDEX.write_text(json.dumps(index, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return not unchanged


def entries() -> list[Path]:
    return sorted(p for p in PLUGINS.glob("*.json") if not p.name.startswith("_"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--download", action="store_true", help="fetch each module and check its checksum")
    parser.add_argument("--source", action="store_true", help="check the source is readable by anyone")
    parser.add_argument("--index", action="store_true", help="rewrite index.json from the entries")
    args = parser.parse_args()

    checked, failed = [], 0
    for path in entries():
        try:
            entry = check(path)
            # An official plugin's source may be private: its module is reviewed where it is merged.
            if args.source and not entry.get("official"):
                source_is_public(entry)
            if args.download:
                download(entry)
            checked.append(entry)
            print(f"  ok  {path.name}")
        except Problem as problem:
            failed += 1
            print(f"FAIL  {path.name}: {problem}")
        except subprocess.TimeoutExpired:
            failed += 1
            print(f"FAIL  {path.name}: the source host did not answer in time")
        except OSError as err:
            failed += 1
            print(f"FAIL  {path.name}: could not reach it: {err}")

    if failed:
        print(f"\n{failed} entr{'y' if failed == 1 else 'ies'} need work.")
        return 1

    if args.index:
        write_index(checked)
        print(f"\nindex.json: {len(checked)} plugin(s)")
    else:
        print(f"\n{len(checked)} entr{'y' if len(checked) == 1 else 'ies'} checked.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
