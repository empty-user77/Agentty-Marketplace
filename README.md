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
  from, and build it again.

## What is where

| | |
|---|---|
| `plugins/<id>.json` | one entry per plugin — what Agentty reads |
| `index.json` | every entry, rebuilt by `scripts/validate.py --index` |
| `sdk/rust/` | the Rust SDK a plugin is written against |
| `src/<id>/` | the plugins published from this repository |
| `modules/<id>-<version>.wasm` | their built modules, which their entries point at |
| `scripts/validate.py` | the checks; `scripts/build-plugins.sh` builds everything in `src/` |

A plugin of your own does not go in `src/`: it lives in your repository, and only its entry comes
here.

## Submitting a plugin

1. Build the module and publish it — a GitHub release of your plugin's repository is the usual
   place.
2. Copy `plugins/_template.json` to `plugins/<your-plugin-id>.json` and fill it in. The id matches
   the file name and the `id` in your `agentty-plugin.json`.
3. Open a pull request. CI checks the entry, downloads the module, and refuses it unless the
   checksum matches.

```sh
python3 scripts/validate.py                 # every entry
python3 scripts/validate.py --download      # also fetch each module and check its checksum
python3 scripts/validate.py --index         # rebuild index.json (CI does this on main)
```

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
  "keywords": ["example"],
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
| `name`, `version`, `description` | shown in Agentty; `version` is `major.minor.patch` |
| `publisher`, `license` | who made it, and under what licence |
| `source` | the public repository the module is built from — **required** |
| `homepage`, `keywords`, `icon` | optional; the icon is a name from Agentty's set |
| `surface` | where its icon sits: `sidebar`, `pane` (default) or `status` |
| `mode` | how its panel opens: `push` (default), `overlay`, `window` or `full` |
| `permissions` | what it asks for — Agentty shows these before anyone installs it |
| `module.url` | `https://` on github.com; it must contain the version, so a release cannot be swapped underneath |
| `module.sha256` | the module's checksum; Agentty refuses a download that does not match |
| `module.size` | its size in bytes (8 MB at most) |

## Updating a plugin

Change `version`, `module.url`, `module.sha256` and `module.size` in your entry and open another
pull request. Agentty offers the update to everyone who has it installed.

## What gets a plugin refused

- A module that is not built from the repository in `source`, or a repository nobody can read.
- A checksum that does not match what the URL serves.
- Permissions the plugin does not use, or a description that does not say what it does with them.
  `net.request` together with `session.read` or `workspace.read` means the plugin can read your
  work and send it somewhere: say plainly, in the description, why it needs both.
- Anything that pretends to be another plugin, another publisher, or Agentty itself.

## Private plugins

Nothing here is required to use a plugin. **Plugins → Install from Folder…** takes any folder, and
**Install from Git** takes any repository you can clone. A plugin you keep to yourself, or to your
company, never has to pass through this list.
