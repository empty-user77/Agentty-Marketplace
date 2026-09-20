# Adding a plugin

## What is offered here

A plugin in this list is a **WebAssembly module** whose **source is public**. Agentty runs the
module itself, so it reaches only what the plugin protocol gives it — no files, no processes, no
network of its own — and everything it does ask for is a permission the user sees before
installing. That is what makes it safe to install a binary from a list.

Plugins that run as a program (`node`, `python`, an executable) have everything you have. Agentty
installs those from a folder or a Git repository, where the person doing it chose the source
themselves; they are not offered here.

## Steps

1. **Build and publish the module.**

   ```sh
   cargo build --release --target wasm32-unknown-unknown
   cp target/wasm32-unknown-unknown/release/your_plugin.wasm your-plugin.wasm
   shasum -a 256 your-plugin.wasm
   wc -c your-plugin.wasm
   ```

   Attach `your-plugin.wasm` to a release of your repository, tagged with the version.

2. **Write the entry.** Copy `plugins/_template.json` to `plugins/<your-plugin-id>.json`. The id
   matches the file name and the `id` in your `agentty-plugin.json`.

3. **Check it.**

   ```sh
   python3 scripts/validate.py --download
   python3 scripts/validate.py --index      # index.json, which Agentty reads
   ```

4. **Open a pull request** with both files. CI runs the same checks.

## Reviewing (for maintainers)

Read the plugin's source, not only its entry:

- Does the module in the release come from that source? Build it and compare what it does, not its
  bytes — a wasm build is not reproducible across machines.
- Does it ask for permissions it does not use? `net.request` with `session.read` or
  `workspace.read` can carry someone's work away: the description must say why both are needed.
- Does the name, publisher or description pretend to be someone else, or Agentty itself?
- Is the licence there, and does the repository carry it?

## Updating a plugin

Change `version`, `module.url`, `module.sha256` and `module.size`. Everyone who installed it is
offered the new version.

## Removing a plugin

Delete the entry. Agentty stops offering it; it stays installed for people who have it, and they
can remove it from the Plugins page.
