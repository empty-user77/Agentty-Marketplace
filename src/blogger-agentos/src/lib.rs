//! Blogger AgentOS: the smallest AgentOS worth reading.
//!
//! Four steps — outline, draft, edit, publish-ready — each one asked of a Claude Code session the
//! user can watch and take over, each answer checked before the next step is sent. The plugin
//! itself writes nothing: it has no files, no network and no browser. It holds the prompts and
//! the rules, and `agentty_plugin::agentos` walks them.
//!
//! What a real AgentOS adds to this is better prompts, not more code.

use agentty_plugin::agentos::{self, Approval, Machine, Step, Workflow};
use agentty_plugin::text::t;
use agentty_plugin::{export_plugin, Host, PaneStatus, Plugin, UiEvent};
use serde_json::Value;

/// The storage key the run is kept under, so it survives a restart of Agentty.
const RUN: &str = "run";

const OUTLINE: &str = "\
You are writing a blog post about: {input}

Give me an outline and nothing else: a working title, then three to six sections, each one a
heading and one line saying what it covers. No prose around it, no draft yet.
Write it in the language the topic is written in.";

const DRAFT: &str = "\
Write the post from this outline:

{step.outline}

Every section from the outline, in the same order, with real sentences under each heading — not
notes. Between 500 and 1500 words. No placeholder text, no \"TODO\", no \"[insert …]\".
Write it in the language of the outline.";

const EDIT: &str = "\
Edit this draft:

{step.draft}

Cut what repeats, make the sentences shorter where they can be, and keep the structure. Write out
the whole edited post, not a list of changes.";

const READY: &str = "\
Save the post as a Markdown file in the current folder, named after its title, with the edited
text below, and tell me the path you wrote:

{step.edit}";

fn has_headings(text: &str) -> Result<(), String> {
    let headings = text.lines().filter(|line| line.trim_start().starts_with('#') || line.trim_end().ends_with(':')).count();
    if headings >= 3 {
        Ok(())
    } else {
        Err("it has fewer than three sections".to_string())
    }
}

fn long_enough(text: &str) -> Result<(), String> {
    let words = text.split_whitespace().count();
    if words < 300 {
        return Err(format!("it is {words} words, and a post is at least 300"));
    }
    no_placeholders(text)
}

fn no_placeholders(text: &str) -> Result<(), String> {
    let lower = text.to_lowercase();
    for marker in ["todo", "[insert", "lorem ipsum", "your text here", "tbd"] {
        if lower.contains(marker) {
            return Err(format!("it still says \"{marker}\""));
        }
    }
    Ok(())
}

fn names_a_file(text: &str) -> Result<(), String> {
    if text.contains(".md") {
        Ok(())
    } else {
        Err("it does not say which file it wrote".to_string())
    }
}

static BLOGGER: Workflow = Workflow {
    id: "blogger",
    title: "Blog post",
    agent: Some("claude"),
    label: ["Blog post", "블로그 글", "ブログ記事", "博客文章"],
    glossary: &[],
    steps: &[
        Step { id: "outline", title: ["Outline", "개요", "アウトライン", "大纲"], prompt: OUTLINE, check: has_headings, approval: Approval::Auto },
        Step { id: "draft", title: ["Draft", "초고", "下書き", "初稿"], prompt: DRAFT, check: long_enough, approval: Approval::Auto },
        // The runner makes this one ask anyway — it is what the step after it writes to disk.
        Step { id: "edit", title: ["Edit", "다듬기", "推敲", "润色"], prompt: EDIT, check: no_placeholders, approval: Approval::Ask },
        Step { id: "save", title: ["Save", "저장", "保存", "保存"], prompt: READY, check: names_a_file, approval: Approval::Ask },
    ],
};

struct Blogger {
    machine: Machine,
    typed: String,
    loading: Option<u64>,
}

impl Default for Blogger {
    fn default() -> Self {
        Self { machine: Machine::new(&BLOGGER), typed: String::new(), loading: None }
    }
}

impl Blogger {
    fn draw(&self, host: &Host) {
        let lang = host.language();
        let ask = t(lang, ["What is the post about?", "무엇에 대한 글인가요?", "何についての記事ですか？", "这篇文章写什么？"]);
        host.set_panel(agentos::panel_in(&self.machine, &self.typed, ask, lang));
    }

    fn keep(&self, host: &Host) {
        host.storage_set(RUN, self.machine.saved());
    }
}

impl Plugin for Blogger {
    fn init(&mut self, host: &Host, _info: &Value) {
        if let Err(why) = BLOGGER.checked() {
            host.log(format!("the workflow is wrong: {why}"));
        }
        // Whatever run was going when Agentty last closed.
        self.loading = Some(host.storage_get(RUN));
    }

    fn command(&mut self, host: &Host, _command: &str) {
        host.show_panel();
    }

    fn panel_open(&mut self, host: &Host) {
        self.draw(host);
    }

    fn ui_event(&mut self, host: &Host, event: UiEvent) {
        if agentos::handle(&mut self.machine, host, &event, &mut self.typed) {
            self.keep(host);
            self.draw(host);
        }
    }

    fn pane_status(&mut self, host: &Host, status: PaneStatus) {
        self.machine.pane_status(host, &status);
        self.keep(host);
        self.draw(host);
    }

    fn answer(&mut self, host: &Host, id: u64, result: Result<Value, String>) {
        if self.loading == Some(id) {
            self.loading = None;
            if let Ok(value) = &result {
                self.machine.restore(&value["value"]);
                self.machine.resume(host);
            }
            return self.draw(host);
        }
        if self.machine.answer(host, id, &result) {
            self.keep(host);
            self.draw(host);
        }
    }
}

export_plugin!(Blogger);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workflow_is_one_a_run_can_be_stopped_in() {
        BLOGGER.checked().expect("the workflow is well formed");
    }

    #[test]
    fn an_outline_needs_sections_and_a_draft_needs_words() {
        assert!(has_headings("# One\n# Two\n# Three").is_ok());
        assert!(has_headings("# One\n# Two").is_err());
        assert!(long_enough(&"word ".repeat(400)).is_ok());
        assert!(long_enough("too short").is_err());
    }

    #[test]
    fn what_the_agent_left_unfinished_is_caught() {
        for half_done in ["TODO: write this", "[insert the example]", "Lorem ipsum dolor", "TBD"] {
            assert!(no_placeholders(half_done).is_err(), "{half_done}");
        }
        assert!(no_placeholders("A finished paragraph about something.").is_ok());
        assert!(names_a_file("I wrote it to ./posts/a-title.md").is_ok());
        assert!(names_a_file("Done.").is_err());
    }
}
