#!/usr/bin/env python3
"""Checks the marketplace entries, and rebuilds index.json from them.

Every entry is a plugin someone submitted: treat it as text from a stranger. Nothing here runs a
module or trusts a URL to be what it says — the checksum is what decides, and the checks below are
what a reviewer would otherwise have to remember.

    python3 scripts/validate.py              # shape, ids, URLs, permissions
    python3 scripts/validate.py --download   # also fetch each module and check its checksum
    python3 scripts/validate.py --index      # rewrite index.json from plugins/*.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
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
PLUGIN_API_VERSION = 2
ID = re.compile(r"^[a-z0-9][a-z0-9-]{1,38}[a-z0-9]$")
VERSION = re.compile(r"^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
ICON = re.compile(r"^[a-z0-9-]{1,40}$")

# What Agentty knows how to ask the user about. A plugin naming anything else would be installed
# with a permission nobody can explain.
PERMISSIONS = {"net.request", "prompt.inject", "terminal.write", "session.read", "workspace.read"}
SURFACES = {"sidebar", "pane", "status"}
MODES = {"push", "overlay", "window", "full"}

# A module is served from a release of a repository, not from someone's server: a release asset
# cannot be replaced without the URL changing, and these hosts are the ones GitHub serves them on.
MODULE_HOSTS = {"github.com", "raw.githubusercontent.com", "objects.githubusercontent.com"}
SOURCE_HOSTS = {"github.com", "gitlab.com", "codeberg.org", "git.sr.ht"}

MAX_MODULE_BYTES = 8 * 1024 * 1024
MAX_DESCRIPTION = 300
MAX_NAME = 60
MAX_KEYWORDS = 10

REQUIRED = ["id", "name", "version", "description", "publisher", "license", "source", "module"]


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


def check(path: Path) -> dict:
    """One entry, checked. Raises Problem with what is wrong."""
    try:
        entry = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        raise Problem(f"not valid JSON: {err}") from err
    if not isinstance(entry, dict):
        raise Problem("an entry is a JSON object")

    for field in REQUIRED:
        if field not in entry:
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
    if host not in SOURCE_HOSTS:
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

    # A plugin that both reads the user's work and sends requests out can carry it away. It is
    # allowed, and it is said out loud.
    if "net.request" in permissions and {"session.read", "workspace.read"} & set(permissions):
        if "net.request" not in entry.get("description", "") and len(entry["description"]) < 40:
            raise Problem("this plugin can read the user's work and send requests: say why in the description")
    return entry


def download(entry: dict) -> None:
    """Fetches the module and checks it against the entry. Never runs it."""
    module = entry["module"]
    request = urllib.request.Request(module["url"], headers={"User-Agent": "agentty-marketplace-validate"})
    with urllib.request.urlopen(request, timeout=60) as response:  # noqa: S310 - https is checked above
        data = response.read(MAX_MODULE_BYTES + 1)
    if len(data) > MAX_MODULE_BYTES:
        raise Problem("the module is larger than the limit")
    if len(data) != module["size"]:
        raise Problem(f"module.size says {module['size']}, the download is {len(data)} bytes")
    digest = hashlib.sha256(data).hexdigest()
    if digest != module["sha256"]:
        raise Problem(f"module.sha256 says {module['sha256']}, the download is {digest}")
    if not data.startswith(b"\x00asm"):
        raise Problem("that file is not a WebAssembly module")


def entries() -> list[Path]:
    return sorted(p for p in PLUGINS.glob("*.json") if not p.name.startswith("_"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--download", action="store_true", help="fetch each module and check its checksum")
    parser.add_argument("--index", action="store_true", help="rewrite index.json from the entries")
    args = parser.parse_args()

    checked, failed = [], 0
    for path in entries():
        try:
            entry = check(path)
            if args.download:
                download(entry)
            checked.append(entry)
            print(f"  ok  {path.name}")
        except Problem as problem:
            failed += 1
            print(f"FAIL  {path.name}: {problem}")
        except OSError as err:
            failed += 1
            print(f"FAIL  {path.name}: could not fetch the module: {err}")

    if failed:
        print(f"\n{failed} entr{'y' if failed == 1 else 'ies'} need work.")
        return 1

    if args.index:
        index = {
            "apiVersion": API_VERSION,
            "updated": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "plugins": checked,
        }
        INDEX.write_text(json.dumps(index, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"\nindex.json: {len(checked)} plugin(s)")
    else:
        print(f"\n{len(checked)} entr{'y' if len(checked) == 1 else 'ies'} checked.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
