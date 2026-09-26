# Agentty Marketplace

The list of plugins Agentty offers in **Plugins → Marketplace**. One file per plugin, added by a
pull request, checked by CI, and read by Agentty as `index.json`.

It also holds the **Rust SDK** for writing a plugin (`sdk/rust`) and the **plugins Agentty itself
publishes** (`src/`), which are built here and served from `modules/`. Those used to ship inside
Agentty; they are here now, on the same footing as everyone else's.

A plugin here is **a WebAssembly module with its source in the open**. That is the whole rule, and
both halves matter:

- **WebAssembly**, because Agentty runs it itself and it reaches only what the protocol gives it —
  no files, no processes, no network of its own. A plugin that runs as a program (`node`, `python`,
  an executable) has everything you have, and Agentty will not install one from a list on the
  internet. Distribute those yourself; people install them from a folder or a Git repository and
  decide for themselves.
- **Source in the open**, because the module here is a binary. Anyone can read what it is built
  from, and build it again — and CI does exactly that before the plugin is accepted. Every entry
  names the commit its module is built from; CI clones it, builds it in a pinned container, and
  refuses the entry unless the bytes that come out hash to the checksum the entry claims. A module
  that cannot be reproduced from its own source is not offered here, whatever else it says.

## What is where

| | |
|---|---|
| `plugins/<id>.json` | one entry per plugin — what Agentty reads |
| `index.json` | every entry, rebuilt by `scripts/validate.py --index` |
| `sdk/rust/` | the Rust SDK a plugin is written against |
| `src/<id>/` | the plugins published from this repository |
| `modules/<id>-<version>.wasm` | their built modules, which their entries point at |
| `scripts/validate.py` | the checks on an entry: shape, hosts, permissions, the build block |
| `scripts/verify_build.py` | builds each entry's source again and compares it to `module.sha256` |
| `scripts/build-plugins.sh` | builds everything in `src/`, in the same pinned container |

A plugin of your own does not go in `src/`: it lives in your repository, and only its entry comes
here.

## Submitting a plugin

1. Build the module and publish it — a GitHub release of your plugin's repository is the usual
   place.
2. Copy `plugins/_template.json` to `plugins/<your-plugin-id>.json` and fill it in. The id matches
   the file name and the `id` in your `agentty-plugin.json`.
3. Open a pull request. CI checks the entry, downloads the module, **builds your source again from
   the commit you named**, and refuses it unless all three agree.

```sh
python3 scripts/validate.py                 # every entry
python3 scripts/validate.py --download      # also fetch each module and check its checksum
python3 scripts/validate.py --source        # also check the source is readable by anyone
python3 scripts/verify_build.py             # build each source again; compare it to the checksum
python3 scripts/validate.py --index         # rebuild index.json (CI does this on main)
```

`verify_build.py` needs Docker: the module is built in a digest-pinned `rust` image so the bytes do
not depend on the machine doing the building. That is what lets CI arrive at the same checksum you
did — see [Reproducible builds](#reproducible-builds).

## What an entry looks like

```json
{
  "id": "hello-world",
  "name": "Hello World",
  "version": "0.1.0",
  "publisher": "Your Name",
  "description": "One sentence about what it does.",
  "icon": "sparkles",
  "license": "MIT",
  "source": "https://github.com/you/agentty-hello-world",
  "build": {
    "repository": "https://github.com/you/agentty-hello-world",
    "rev": "3f2b1c9e4a7d05b8c6e1f0a2d4b83c7e9015d6af",
    "path": ".",
    "toolchain": "1.98.1",
    "artifact": "target/wasm32-unknown-unknown/release/hello_world.wasm"
  },
  "keywords": ["example"],
  "apiVersion": 1,
  "surface": "sidebar",
  "mode": "push",
  "permissions": [],
  "module": {
    "url": "https://github.com/you/agentty-hello-world/releases/download/v0.1.0/hello-world.wasm",
    "sha256": "…64 hex characters…",
    "size": 93292
  }
}
```

| Field | |
|---|---|
| `id` | 2–40 characters, `a-z 0-9 -`; the file is `plugins/<id>.json` |
| `name`, `version`, `description` | shown in Agentty; `version` is `major.minor.patch`, the name is up to 60 characters and the description up to 300 |
| `publisher`, `license` | who made it, and under what licence |
| `source` | the public repository the module is built from — **required**, on `github.com`, `gitlab.com`, `codeberg.org` or `git.sr.ht`. It has to be the same repository as `build.repository`, so the code an entry links to is the code it ships |
| `build` | **required** — how to build that module again. `repository` (the clone URL), `rev` (the full 40-character commit, not a tag or a branch, which can be moved afterwards), `path` (the plugin's directory in the repository, or `.`), `toolchain` (the Rust release the marketplace builds with) and `artifact` (the `.wasm` the build writes, relative to `path`). Nothing here is a command: what is run on it is fixed in `scripts/verify_build.py` |
| `homepage`, `keywords`, `icon` | optional; the icon is a name from Agentty's set. A plugin that wants its own artwork carries it in the module — see [Giving a plugin a logo](#giving-a-plugin-a-logo) — an entry has no `logo` field |
| `apiVersion` | the plugin protocol the module is built against; leave it out for `1`. Agentty tells anyone running an older version that they need to update, instead of installing something it cannot run |
| `surface` | where its icon sits: `sidebar`, `pane` (default) or `status` |
| `mode` | how its panel opens: `push` (default), `overlay`, `window` or `full` |
| `permissions` | what it asks for — Agentty shows these before anyone installs it |
| `module.url` | `https://` on `github.com`, `raw.githubusercontent.com` or `objects.githubusercontent.com`. Put the version in the path so a release cannot be swapped underneath — that is a convention, not something CI checks |
| `module.sha256` | the module's checksum. Agentty refuses a download that does not match, and refuses bytes that are not a WebAssembly module even when it does |
| `module.size` | its exact length in bytes, up to 8 MB. Not a ceiling: a download of any other length is refused, so this changes with every build |

## Giving a plugin a logo

Agentty draws a plugin's own artwork instead of its `icon` name wherever the plugin appears. An
entry does not carry one and Agentty never fetches one from an address: the picture travels inside
the module, in a WebAssembly custom section called `agentty.logo`. The engine ignores that section,
`module.sha256` already covers it, and CI builds it again from the source the entry names — so the
picture that is drawn is the picture that was reviewed, and installing a plugin tells its author
nothing.

In Rust it is a static with two attributes. `#[used]` is not optional: without it a release build
drops a static nothing refers to, and the section goes with it.

```rust
#[used]
#[link_section = "agentty.logo"]
static LOGO: [u8; 4096] = *include_bytes!("logo.png");
```

The length has to match the file. Agentty keeps the picture only when it is a **PNG, JPEG, GIF or
WebP**, actually is one (the bytes are checked, not a name), and is **512 KB at most**; otherwise the
plugin keeps its `icon`. **An SVG is never drawn.** An SVG is a document, not a picture: the
renderer resolves the addresses inside it, and one of those could be a file on the machine of
whoever installed the plugin. Export a raster image instead.

## Updating a plugin

Change `version`, `module.url`, `module.sha256` and `module.size` in your entry and open another
pull request. `module.size` is the new module's exact length, not the old one's. Agentty offers
the update to everyone who has it installed.

If the new version uses something only a newer Agentty has, raise `apiVersion` with it. People on an
older Agentty then keep the version they have and are told to update, instead of being handed a
module their app cannot run.

## Reproducible builds

An entry is a link to a binary. Reading the source beside it tells you nothing about that binary —
the two are only connected by whoever uploaded them. So CI connects them itself:

1. It clones `build.repository` at `build.rev`. A commit, not a tag: a tag can be pointed at other
   code the day after the review, and a branch moves on its own.
2. It reads `agentty-plugin.json` at `build.path` and checks it agrees with the entry — same id,
   same version, same `apiVersion`, the same permissions. The entry is what Agentty shows people;
   the manifest is what the plugin actually is, and they are not allowed to disagree.
3. It builds it with `cargo build --release --locked --offline --target wasm32-unknown-unknown`,
   inside a `rust` image pinned by digest, with `RUSTUP_TOOLCHAIN` forced to the marketplace's Rust
   release — so a `rust-toolchain.toml` in the submission cannot choose its own compiler, and with
   it its own bytes. Dependencies are fetched first, under `Cargo.lock`; the build itself then runs
   with **no network at all**, because a build script is code and has no business reaching out.
4. It hashes the `.wasm` the build wrote. If that is not `module.sha256`, the entry is refused.

Two consequences worth knowing before you submit:

- **`Cargo.lock` has to be committed.** Without it the dependencies, and so the module, can change
  under an entry that was already reviewed.
- **The toolchain is the marketplace's, not yours.** `build.toolchain` must be the Rust release in
  `BUILD_TOOLCHAIN` (`scripts/validate.py`). Build with the same one — `scripts/build-plugins.sh`
  does it for you — or your checksum will not be the one CI arrives at. When the marketplace raises
  that release, every module is rebuilt and re-checksummed at once.

This runs once, at the pull request that registers or updates an entry — a pull request rebuilds
only the entries it touches, never the whole list on a schedule, so the cost of checking a plugin
stays the same no matter how many others are already in the marketplace. It is not repeated for an
entry that already merged: a source that is deleted, rewritten or made private afterwards is not
caught by this list on its own.

## What gets a plugin refused

- A module that does not come out of its own source: CI builds `build.rev` and gets other bytes.
- A `build.rev` that is not in the repository, a tag or branch where a commit belongs, or a
  repository that cannot be cloned without credentials.
- An `agentty-plugin.json` that disagrees with the entry about the version, the protocol or the
  permissions.
- A missing `Cargo.lock`, or a build that does not complete with the marketplace's Rust release.
- A module that is not built from the repository in `source`, or a repository nobody can read.
- A checksum that does not match what the URL serves.
- Permissions the plugin does not use, or a description that does not say what it does with them.
  `net.request` together with `session.read` or `workspace.read` means the plugin can read your
  work and send it somewhere: say plainly, in the description, why it needs both.
- Anything that pretends to be another plugin, another publisher, or Agentty itself.

## Official plugins

The marketplace's own plugins are marked `"official": true`. What makes one official is where its
module is: a file in `modules/` of this repository, which `module.url` points at. Only whoever can
merge here can put one there, so an entry that merely says it is official is refused.

An official plugin needs no `build` block and its `source` may be private: its module is reviewed
where it is merged, not rebuilt from a public commit. Everything else is checked as for any entry —
its shape, permissions, size and checksum against the file in `modules/`.

## Private plugins

Nothing here is required to use a plugin. **Plugins → Install from Folder…** takes any folder, and
**Install from Git** takes any repository you can clone. A plugin you keep to yourself, or to your
company, never has to pass through this list.
