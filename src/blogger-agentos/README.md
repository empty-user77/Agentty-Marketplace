# Blogger AgentOS

The smallest AgentOS worth reading: a blog post in four steps.

Outline, draft, edit, save. Each step sends a prompt to a Claude Code session that opens beside
the panel, waits for it to stop, reads what the agent wrote, and checks it — three sections in the
outline, three hundred words in the draft, nothing left saying `TODO` — before the next step is
sent. You read the edited post before it is written to disk.

## What is in it

About 180 lines, and most of them are prompts. The state machine — send, wait, read, check, go on
or ask — is `agentty_plugin::agentos`, which every AgentOS shares. Writing another one is a list
of steps and the prompts for them.

```rust
static BLOGGER: Workflow = Workflow {
    id: "blogger",
    title: "Blog post",
    agent: Some("claude"),
    glossary: &[],
    steps: &[
        Step { id: "outline", title: "Outline", prompt: OUTLINE, check: has_headings, approval: Approval::Auto },
        Step { id: "draft", title: "Draft", prompt: DRAFT, check: long_enough, approval: Approval::Auto },
        Step { id: "edit", title: "Edit", prompt: EDIT, check: no_placeholders, approval: Approval::Ask },
        Step { id: "save", title: "Save", prompt: READY, check: names_a_file, approval: Approval::Ask },
    ],
};
```

`{input}` is what you typed; `{step.outline}` is what the outline step produced. A check that
fails sends the agent what is missing and asks again, three times, and then stops and says so.

## The permissions

`prompt.inject`, `session.read`, `workspace.read`. It asks an agent to write, reads what the agent
wrote, and hears when the session has stopped. It has no files, no network and no browser of its
own — the agent has those, in a session you can read.

## Building it

```sh
./build.sh
```

Then **Plugins → Install from folder** and pick this folder.

Needs Agentty with plugin protocol 2 (`host/timer` and `pane/status`).
