# agentty-plugin (Rust SDK)

Write an Agentty plugin in Rust and ship it as one `.wasm` file that runs on macOS, Windows and
Linux.

Agentty runs the module itself, on an interpreter, and hands it three functions: send a message,
write a log line, read the clock. There is nothing else — no file, no socket, no environment
variable, no process — so a plugin reaches only what the protocol gives it, and only with the
permissions its `agentty-plugin.json` declares.

## A plugin

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
agentty-plugin = { path = "../../sdk/rust" }

[profile.release]
opt-level = "z"
lto = true
panic = "abort"
strip = true
```

```rust
use agentty_plugin::{export_plugin, ui, Host, Plugin, UiEvent};

#[derive(Default)]
struct Hello {
    clicks: u32,
}

impl Plugin for Hello {
    fn panel_open(&mut self, host: &Host) {
        host.set_panel(ui::column(vec![
            ui::text(format!("Clicked {} times", self.clicks)),
            ui::button("go", "Click me"),
        ]));
    }

    fn ui_event(&mut self, host: &Host, event: UiEvent) {
        if event.element == "go" {
            self.clicks += 1;
            self.panel_open(host);
        }
    }
}

export_plugin!(Hello);
```

```json
{
  "id": "hello",
  "name": "Hello",
  "version": "0.1.0",
  "runtime": "wasm",
  "main": "hello.wasm",
  "contributes": { "panel": { "title": "Hello", "surface": "sidebar" } }
}
```

## Build and install

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/hello.wasm hello.wasm
```

Put the module next to `agentty-plugin.json`, then **Plugins → Install from Folder…** in Agentty
and pick the folder. **Restart** on the plugin's page picks up a new build.

## What the host gives you

| `Host` | |
|---|---|
| `set_panel(tree)` · `show_panel()` | the panel, built with `ui::` |
| `notify_user(kind, message)` · `set_badge(text)` | a toast, and up to 8 characters on the icon |
| `copy(text)` | puts text on the clipboard |
| `log(line)` | the plugin's log on the Plugins page |
| `open_url(url)` | a page in the user's browser |
| `fetch(request)` | an HTTP request — needs `net.request` |
| `prompt(text, target)` | a prompt for an agent — needs `prompt.inject` |
| `start_session(title, agent, text)` · `prompt_pane(pane, text)` | open a session for a piece of work, and send it the next prompt |
| `session(pane, max_turns)` | what an agent wrote — needs `session.read` |
| `wait(ms)` | comes back later; a module has no clock and no loop of its own |
| `storage_get(key)` · `storage_set(key, value)` | the plugin's own folder, no permission needed |
| `call(method, params)` · `notify(method, params)` | anything else in the protocol |
| `context()` · `language()` · `now_ms()` | where the user is, what they read, and the clock |

`call`, `fetch`, `session` and `wait` return a request id; the answer arrives in `Plugin::answer`.
`Plugin::pane_status` is told how a session this plugin started is getting on.

## Four languages

Agentty is in English, Korean, Japanese and Chinese, and a plugin that is only in one of them is
the odd one out in the window. Give each string four ways:

```rust
use agentty_plugin::text::t;

let lang = host.language();
ui::button("send", t(lang, ["Send", "보내기", "送信", "发送"]))
```

A language left empty reads English, so a plugin can gain one at a time. Prompts sent to an agent
are a different thing: those stay in English and tell the agent which language to answer in.

## Running work through agents

`agentty_plugin::agentos` is for a plugin whose job is to walk a piece of work through Agentty's
agents — see [`docs/plugins/agentos.md`](https://github.com/empty-user77/Agentty/blob/main/docs/plugins/agentos.md).
A workflow is a list of steps; a step is a prompt, a check on what comes back, and whether the
user is asked before the next one.

```rust
static BLOG: Workflow = Workflow {
    id: "blog",
    title: "Blog post",                                   // names the session; an agent reads it
    label: ["Blog post", "블로그 글", "ブログ記事", "博客文章"],  // what the user sees
    agent: Some("claude"),
    glossary: &[],                                        // prompt pieces several steps share
    steps: &[
        Step { id: "outline", title: ["Outline", "개요", "アウトライン", "大纲"],
               prompt: OUTLINE, check: has_sections, approval: Approval::Auto },
        Step { id: "draft", title: ["Draft", "초고", "下書き", "初稿"],
               prompt: DRAFT, check: long_enough, approval: Approval::Ask },
    ],
};
```

The machine sends a step, waits for the session to stop, reads what the agent wrote, checks it,
and either goes on or sends the agent what is missing — three times, then it stops and says why.
`Machine::saved()` is the run, for a storage key; `restore` and `resume` pick it up after a
restart. `agentos::panel_in(&machine, typed, prompt, lang)` draws the whole thing.

**The user is asked before the last step, whatever that step says** — the last step is the one
that acts on the world, and what the user is shown is what it will act on. `Workflow::checked()`
refuses a workflow that could not stop there, in `init` rather than at the moment it would have
posted.

## Testing one

Built for anything but wasm, the three host functions keep what they were given instead of
dropping it, so a plugin runs in an ordinary `cargo test`:

```rust
let host = Host::new();
let mut plugin = MyPlugin::default();
plugin.panel_open(&host);
let sent = agentty_plugin::host_stubs::taken();   // what it asked Agentty for, as JSON
assert_eq!(sent[0]["method"], "ui/setPanel");
```

## Limits

One message is handled at a time, with a budget of work behind it: a plugin that never returns is
stopped, and so is one that sends more than 256 messages while handling a single event. Memory is
capped at 64 MB and a message at 16 MB.

The protocol, including the module's ABI, is in `docs/plugins/protocol.md`.
