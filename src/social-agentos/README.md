# Social AgentOS

Posting, run through agents you can watch.

Three workflows — a post for X, replies to a conversation, an Instagram caption — each one a few
steps. A step sends a prompt to a Claude Code session that opens beside the panel, waits for that
session to stop, reads what the agent wrote, and checks it before the next step is sent. You read
the draft before anything is published, every time.

## How it can post at all

It cannot. The plugin is a WebAssembly module: no files, no network, no browser. What it has is
`prompt.inject`, `session.read` and `workspace.read` — it can ask an agent to do something, read
what the agent wrote, and see how the session is getting on.

The agent has the rest. Agentty gives every session a terminal and its own browser, driven with
`agentty browser navigate | click | type | text | screenshot`, and that browser is the one you are
already signed in to. So the plugin writes the prompt and the agent does the work, in a tab you
can read, take over, or close.

That is why this needs no permission beyond the three above, and why there is nowhere for it to
keep a password: it never has one.

## The workflows

| | Steps |
|---|---|
| **Post to X** | read what is being said → draft → **you read it** → post it |
| **Reply on X** | find conversations worth joining → draft the replies → **you read them** → send |
| **Instagram caption** | read how the account writes → caption → **you read it** → save it, and the caption goes on your clipboard |

## What it will not do

- **Post anything you have not read.** The step before the last always waits for you, and the
  runner does not let a workflow opt out of that — it is not a setting.
- **Sign in.** Every prompt that opens a page says so: if a site asks for a login or a captcha,
  the agent stops and tells you what it saw. It does not type passwords and it has none to type.
- **Act while it is reading.** The gathering steps say, in as many words, not to like, repost,
  reply or follow while they look.
- **Send more than five replies in a run**, and each one is checked for length before it goes.

Instagram needs the picture chosen by hand, so the last step stops there: it writes the caption to
`instagram-caption.txt`, puts it on your clipboard and opens Instagram for you.

## Using it

Open the panel, pick a workflow, say what it is about, press **Start**. The session opens beside
it. When a step needs you, the panel shows what came back with **Continue**, **Ask again** and
**Stop**. A run is kept between restarts of Agentty: it picks up where it was.

## Building it

```sh
./build.sh
```

Then **Plugins → Install from folder** and pick this folder.

Needs Agentty with plugin protocol 2 (`host/timer` and `pane/status`).
