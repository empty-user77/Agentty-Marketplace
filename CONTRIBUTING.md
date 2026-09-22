# Adding a plugin

## What is offered here

A plugin in this list is a **WebAssembly module** whose **source is public** — on `github.com`,
`gitlab.com`, `codeberg.org` or `git.sr.ht`; it does not have to be GitHub. Agentty runs the
module itself, so it reaches only what the plugin protocol gives it — no files, no processes, no
network of its own — and everything it does ask for is a permission the user sees before
installing. On installing, the download has to be exactly the length the entry claims, hash to the
checksum it claims, and begin like a WebAssembly module. That is what makes it safe to install a
binary from a list.

And the checksum has to be one the source produces. Before a plugin is accepted, CI clones the
commit your entry names, builds it in a container pinned to one Rust release, and refuses the entry
unless the bytes that come out hash to exactly what you wrote in `module.sha256`. "The source is
public" and "this binary is that source" are different claims; only the second one is checkable,
and this is how it is checked.

Plugins that run as a program (`node`, `python`, an executable) have everything you have. Agentty
installs those from a folder or a Git repository, where the person doing it chose the source
themselves; they are not offered here.

## Building the plugins published here

`src/` holds the plugins this repository publishes, written against `sdk/rust`.

```sh
./scripts/build-plugins.sh      # builds every plugin in src/ into modules/, with checksums
python3 scripts/validate.py --index
```

Each writes `modules/<id>-<version>.wasm`; the entry in `plugins/<id>.json` names that file, its
checksum and its size. Raising a version means a new file beside the old one, so an entry always
points at the bytes it was reviewed with.

`build-plugins.sh` needs Docker, and builds in the same pinned container CI uses. Building with
whatever `cargo` is on your machine gives different bytes and a checksum CI will not agree with.

Because an entry names the **commit** it is built from, publishing here is: build, fill in the
entry, push the source, then set `build.rev` to the commit you just pushed and run
`python3 scripts/verify_build.py` to confirm it reproduces.

## Steps

1. **Commit `Cargo.lock` and push.** CI builds with `--locked`: without the lockfile the
   dependencies, and so the module, could change under an entry that was already reviewed. Note
   the commit you pushed — the entry names it, and it has to be a commit anyone can fetch.

2. **Build the module with the marketplace's Rust**, not with whatever is on your machine. The
   toolchain decides the bytes, so this is the difference between a checksum CI agrees with and one
   it does not. `BUILD_TOOLCHAIN` in `scripts/validate.py` says which release that is.

   ```sh
   docker run --rm -v "$PWD:/src" -w /src rust:1.98.1-bookworm bash -c '
     rustup target add wasm32-unknown-unknown >/dev/null
     cargo fetch --locked --target wasm32-unknown-unknown
     cargo build --release --locked --offline --target wasm32-unknown-unknown'
   shasum -a 256 target/wasm32-unknown-unknown/release/your_plugin.wasm
   wc -c target/wasm32-unknown-unknown/release/your_plugin.wasm
   ```

3. **Publish the module.** Attach that `.wasm` to a release of your repository, tagged with the
   version. The URL has to contain the version, so a release cannot be swapped underneath it.

4. **Write the entry.** Copy `plugins/_template.json` to `plugins/<your-plugin-id>.json`. The id
   matches the file name and the `id` in your `agentty-plugin.json` — as do the version, the
   `apiVersion` and the permissions, all of which CI compares. Fill in `build` with the commit from
   step 1, the directory your plugin lives in, and the `.wasm` your build writes.

5. **Check it, the way CI will.**

   ```sh
   python3 scripts/validate.py --download   # the entry, and the module behind it
   python3 scripts/validate.py --source     # the source is readable without credentials
   python3 scripts/verify_build.py          # build it again from your commit; compare (needs Docker)
   python3 scripts/validate.py --index      # index.json, which Agentty reads
   ```

6. **Open a pull request** with both files. CI runs the same checks.

### If the rebuild does not match

`verify_build.py` prints both checksums and what it built. The usual causes, in order:

- a different Rust release — `build.toolchain` has to be the marketplace's, and you have to have
  built with it;
- `build.rev` pointing at a different commit than the one you built, or at a tag;
- an uncommitted `Cargo.lock`, or one that changed after you built;
- the module in the release being an older build than the source at that commit.

## Reviewing (for maintainers)

CI now answers the question that used to need a human: the module in the release is built from the
commit in the entry, or the pull request is red. What is left is what a machine cannot read.

- Does the source do something the description does not admit to? The rebuild proves the binary is
  that source; it says nothing about what the source does.
- Does it ask for permissions it does not use? `net.request` with `session.read` or
  `workspace.read` can carry someone's work away: the description must say why both are needed.
- Does the name, publisher or description pretend to be someone else, or Agentty itself?
- Is the licence there, and does the repository carry it?

## Updating a plugin

Change `version`, `module.url`, `module.sha256`, `module.size` and `build.rev` — the last one is
easy to forget, and an entry still naming the old commit is an entry whose rebuild produces the old
module. Everyone who installed it is offered the new version.

## Removing a plugin

Delete the entry. Agentty stops offering it; it stays installed for people who have it, and they
can remove it from the Plugins page.
