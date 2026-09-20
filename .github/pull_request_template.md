## The plugin

- **Name and id:**
- **Source:** <!-- the public repository the module is built from -->
- **What it does, in a sentence:**

## Before this can be offered to everyone

- [ ] The module is built from the repository in `source`, and anyone can read that repository.
- [ ] `module.url` points at a release asset whose URL contains the version.
- [ ] `module.sha256` and `module.size` match that file (`shasum -a 256 plugin.wasm`, `wc -c`).
- [ ] The plugin is a WebAssembly module — `"runtime": "wasm"` in its `agentty-plugin.json`.
- [ ] `permissions` lists only what the plugin uses, and the description says why it needs them.
- [ ] `python3 scripts/validate.py --download` passes here.

<!-- Updating an existing plugin? Say what changed in this version. -->
