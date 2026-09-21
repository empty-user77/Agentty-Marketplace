//! Social AgentOS: the posting week, run through agents that drive Agentty's own browser.
//!
//! Three workflows — a post for X, replies to a conversation, an Instagram caption — each one a
//! few prompts and the rules for what a good answer looks like. The plugin holds them and nothing
//! else: it has no browser, no network, no files. The agent has all three, because Agentty
//! already gives it a terminal and `agentty browser`, and the browser it drives is the one the
//! user is signed in to.
//!
//! That is the whole trick, and it is why this needs no new permission: what would be a
//! dangerous thing to hand a plugin — the user's logged-in browser — is something the agent
//! already has, in a session the user is watching.
//!
//! Where the user stands in it:
//!
//! - Nothing is posted that the user has not read. The step before the last always waits for
//!   them, and the runner does not let a workflow opt out of that.
//! - The agent never signs in and never touches a password: if a page asks, it stops and says so.
//! - Reading is reading. The gathering steps say, in as many words, not to like, follow, reply or
//!   post while they look.

use agentty_plugin::agentos::{self, Approval, Machine, State, Step, Workflow};
use agentty_plugin::{export_plugin, ui, Host, PaneStatus, Plugin, UiEvent};
use serde_json::Value;

/// Storage keys: the run, and which workflow it belongs to.
const RUN: &str = "run";
const FLOW: &str = "flow";

/// The posting limit a reply run is held to, in the prompt and in the check.
const MAX_REPLIES: usize = 5;
/// What X calls a post, and what the draft is checked against.
const POST_LIMIT: usize = 280;

/// What every step that touches a page is told. Agentty's browser is beside the terminal and is
/// already the user's — signed in, with their cookies — which is exactly why the rules about not
/// signing in and not acting while reading are in here and not left to the model's judgement.
const BROWSER: &str = "\
Use Agentty's in-app browser. It is already open beside your terminal and already signed in as
the user. Drive it from the shell:

  agentty browser navigate <url>           load a page
  agentty browser wait-load                wait until it has finished loading
  agentty browser text [css-selector]      the visible text of the page or one element
  agentty browser elements                 the clickable and typeable elements, with selectors
  agentty browser click <css-selector>     click one
  agentty browser type <css-selector> <text>   put text in a field
  agentty browser press Enter              a key on whatever has focus
  agentty browser screenshot <path.png>    what the page is showing

Rules you do not break:
- Never sign in, never type a password, never accept a login prompt. If a page asks you to sign
  in or to prove you are not a robot, stop and tell me what you saw.
- Do not install anything, and do not use any other browser or any API key to do this.
- If the page does not look like what this asks for, stop and describe it. Do not try another way.";

const GATHER_X: &str = "\
Find out what is being said about: {input}

{browser}

Go to https://x.com/search?q={input}&f=live and wait for it to load. Read the page and list the
ten posts most worth knowing about. For each: the author's handle, one line of what it says, and
the link to it.

You are reading. Do not like, repost, reply to, follow or post anything at all.
If the page shows no posts, say so and stop.";

const DRAFT_X: &str = "\
Write one post for X about {input}.

Here is what is being said already:

{step.gather}

It must be under 280 characters. Say one thing and say it plainly. No hashtags unless one is
doing real work. No emoji unless the surrounding conversation uses them. Do not open the browser
for this step.

Write it in the same language as: {input}
Answer with the text of the post and nothing else — no heading, no explanation, no quotation
marks around it.";

const POST_X: &str = "\
Post this to X, exactly as it is written, changing nothing:

{step.draft}

{browser}

Go to https://x.com/compose/post, wait for it to load, put the text into the composer and send
it. Then read the page and give me the link to the post you just made.

Post it once. If it is already posted, say so instead of posting it again.";

const FIND_REPLIES: &str = "\
Find conversations about {input} that are worth joining.

{browser}

Go to https://x.com/search?q={input}&f=live and wait for it to load. Read the page and pick the
five posts where a reply would actually add something — a question you can answer, a mistake
worth correcting kindly, an experience you can match. For each: the handle, the link, and one
line on what a useful reply would say.

Skip anything that is an argument, an advertisement, or about politics, religion, health advice
or anyone's personal misfortune.

You are reading. Do not like, repost, reply, follow or post while you do this.";

const DRAFT_REPLIES: &str = "\
Write the replies for these:

{step.find}

One reply per post, at most five. Each one under 280 characters, in the language of the post it
answers. Say something a person would say: no compliments about the post itself, no \"great
thread\", no links unless one is the answer. Do not open the browser for this step.

Answer as a numbered list, each entry the link on one line and the reply text on the next, and
nothing else.";

const SEND_REPLIES: &str = "\
Send these replies, each one to the post above it, exactly as written:

{step.draft}

{browser}

For each: open its x.com link, wait for it to load, put the reply in the reply box and send it.
Then move to the next one.

Send each reply once, and no more than five in total. If one fails, say which and go on to the
next. When you are done, list what was sent, with a link to each reply.";

const RESEARCH_IG: &str = "\
Work out what this Instagram post should say: {input}

{browser}

Go to https://www.instagram.com/ and wait for it to load. Read the account's own recent posts —
how long the captions are, whether they use emoji, where the hashtags sit, how the first line
reads. Then say what this post's caption should do and in what voice, in a short paragraph.

You are reading. Do not like, comment, follow or post anything.";

const CAPTION_IG: &str = "\
Write the Instagram caption for: {input}

The account reads like this:

{step.research}

A first line that works on its own, because that is all anyone sees. Then the rest. Then the
hashtags on their own line, between five and fifteen of them, all of them about what the post is
actually about. Do not open the browser for this step.

Write it in the language of: {input}
Answer with the caption and nothing else.";

const SAVE_IG: &str = "\
Save this caption so it is ready to use:

{step.caption}

Write it to a file in the current folder called instagram-caption.txt, and tell me the path.

{browser}

Then open https://www.instagram.com/ so it is ready. Do not try to create the post and do not
upload anything: Instagram wants the picture chosen by hand, and that is the user's to do.";

/// What `{browser}` becomes in every prompt that drives a page.
static BROWSER_RULES: &[(&str, &str)] = &[("browser", BROWSER)];

fn not_empty(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        Err("it is empty".to_string())
    } else {
        Ok(())
    }
}

fn no_placeholders(text: &str) -> Result<(), String> {
    let lower = text.to_lowercase();
    for marker in ["todo", "[insert", "lorem ipsum", "your text here", "xxx"] {
        if lower.contains(marker) {
            return Err(format!("it still says \"{marker}\""));
        }
    }
    Ok(())
}

/// Whether what came back is an answer or the agent talking about what it is about to do.
///
/// Agentty writes a tool call into the conversation as a line of its own, so a session read while
/// an agent is between two of them gives back a sentence and a `[tool: …]`. The runner waits for
/// a stop that lasts before it reads at all; this is what catches the rest.
fn is_an_answer(text: &str) -> Result<(), String> {
    not_empty(text)?;
    let prose: String = text.lines().filter(|line| !line.trim_start().starts_with("[tool:")).collect::<Vec<_>>().join("\n");
    if prose.trim().chars().count() < 200 {
        return Err("it is a sentence about what it is going to do, not what it found".to_string());
    }
    Ok(())
}

/// A page that would not open, a login wall, a captcha: the agent was told to say so, and a step
/// that says so has not done its job.
fn actually_looked(text: &str) -> Result<(), String> {
    let lower = text.to_lowercase();
    for stop in ["sign in", "log in", "not signed in", "captcha", "are not a robot", "no posts"] {
        if lower.contains(stop) {
            return Err(format!("the page asked for something instead of showing posts (\"{stop}\")"));
        }
    }
    is_an_answer(text)
}

fn lists_posts(text: &str) -> Result<(), String> {
    actually_looked(text)?;
    if text.matches("x.com/").count() < 3 {
        return Err("it found fewer than three posts to work from".to_string());
    }
    Ok(())
}

fn fits_a_post(text: &str) -> Result<(), String> {
    not_empty(text)?;
    no_placeholders(text)?;
    let length = text.trim().chars().count();
    if length > POST_LIMIT {
        return Err(format!("it is {length} characters and a post is at most {POST_LIMIT}"));
    }
    // An answer that explains itself instead of being the post.
    if text.lines().count() > 6 {
        return Err("it is an explanation, not a post — answer with the text only".to_string());
    }
    Ok(())
}

fn replies_that_fit(text: &str) -> Result<(), String> {
    not_empty(text)?;
    no_placeholders(text)?;
    let links = text.matches("x.com/").count();
    if links == 0 {
        return Err("it does not say which post each reply answers".to_string());
    }
    if links > MAX_REPLIES {
        return Err(format!("it has {links} replies and at most {MAX_REPLIES} are sent"));
    }
    for line in text.lines() {
        let length = line.trim().chars().count();
        if length > POST_LIMIT {
            return Err(format!("one reply is {length} characters and a reply is at most {POST_LIMIT}"));
        }
    }
    Ok(())
}

fn links_to_what_it_posted(text: &str) -> Result<(), String> {
    not_empty(text)?;
    if text.contains("x.com/") {
        Ok(())
    } else {
        Err("it does not link to what it posted".to_string())
    }
}

fn has_a_caption(text: &str) -> Result<(), String> {
    not_empty(text)?;
    no_placeholders(text)?;
    let tags = text.matches('#').count();
    if !(5..=15).contains(&tags) {
        return Err(format!("it has {tags} hashtags and it should have between 5 and 15"));
    }
    Ok(())
}

fn names_the_file(text: &str) -> Result<(), String> {
    if text.contains("instagram-caption.txt") {
        Ok(())
    } else {
        Err("it does not say where it saved the caption".to_string())
    }
}

static X_POST: Workflow = Workflow {
    id: "x-post",
    title: "Post to X",
    agent: Some("claude"),
    glossary: BROWSER_RULES,
    steps: &[
        Step { id: "gather", title: "Read the room", prompt: GATHER_X, check: lists_posts, approval: Approval::Auto },
        // The runner makes this one ask whatever it says here: it is what gets posted.
        Step { id: "draft", title: "Draft", prompt: DRAFT_X, check: fits_a_post, approval: Approval::Ask },
        Step { id: "post", title: "Post it", prompt: POST_X, check: links_to_what_it_posted, approval: Approval::Ask },
    ],
};

static X_REPLY: Workflow = Workflow {
    id: "x-reply",
    title: "Reply on X",
    agent: Some("claude"),
    glossary: BROWSER_RULES,
    steps: &[
        Step { id: "find", title: "Find conversations", prompt: FIND_REPLIES, check: lists_posts, approval: Approval::Auto },
        Step { id: "draft", title: "Draft replies", prompt: DRAFT_REPLIES, check: replies_that_fit, approval: Approval::Ask },
        Step { id: "send", title: "Send them", prompt: SEND_REPLIES, check: links_to_what_it_posted, approval: Approval::Ask },
    ],
};

static IG_CAPTION: Workflow = Workflow {
    id: "ig-caption",
    title: "Instagram caption",
    agent: Some("claude"),
    glossary: BROWSER_RULES,
    steps: &[
        Step { id: "research", title: "Read the account", prompt: RESEARCH_IG, check: actually_looked, approval: Approval::Auto },
        Step { id: "caption", title: "Caption", prompt: CAPTION_IG, check: has_a_caption, approval: Approval::Ask },
        Step { id: "save", title: "Save it", prompt: SAVE_IG, check: names_the_file, approval: Approval::Ask },
    ],
};

static FLOWS: &[&Workflow] = &[&X_POST, &X_REPLY, &IG_CAPTION];

fn flow_by_id(id: &str) -> &'static Workflow {
    FLOWS.iter().copied().find(|flow| flow.id == id).unwrap_or(&X_POST)
}

struct Social {
    machine: Machine,
    typed: String,
    /// The `storage/get` answers being waited for.
    loading_flow: Option<u64>,
    loading_run: Option<u64>,
    /// The caption of a finished Instagram run, put on the clipboard once.
    copied: bool,
}

impl Default for Social {
    fn default() -> Self {
        Self { machine: Machine::new(&X_POST), typed: String::new(), loading_flow: None, loading_run: None, copied: false }
    }
}

impl Social {
    fn draw(&self, host: &Host) {
        let picker = ui::row(
            FLOWS
                .iter()
                .map(|flow| {
                    let chosen = flow.id == self.machine.flow().id;
                    ui::styled_button(format!("flow.{}", flow.id), flow.title, if chosen { "primary" } else { "ghost" })
                })
                .collect(),
        );
        let run = agentos::panel(&self.machine, &self.typed, prompt_for(self.machine.flow().id));
        host.set_panel(ui::column(vec![picker, ui::divider(), run]));
    }

    fn keep(&self, host: &Host) {
        host.storage_set(RUN, self.machine.saved());
        host.storage_set(FLOW, Value::String(self.machine.flow().id.to_string()));
    }

    /// A finished Instagram run has a caption the user is about to paste.
    fn copy_caption(&mut self, host: &Host) {
        if self.copied || self.machine.run().state != State::Done || self.machine.flow().id != IG_CAPTION.id {
            return;
        }
        if let Some(caption) = self.machine.run().output.get("caption") {
            host.copy(caption.clone());
            host.notify_user("success", "The caption is on the clipboard — attach the picture and paste it.");
            self.copied = true;
        }
    }

    fn pick(&mut self, host: &Host, id: &str) {
        if id == self.machine.flow().id {
            return;
        }
        if self.machine.run().state.is_busy() {
            return host.notify_user("warning", "This run is still going — stop it first.");
        }
        self.machine = Machine::new(flow_by_id(id));
        self.copied = false;
        self.keep(host);
        self.draw(host);
    }
}

fn prompt_for(flow: &str) -> &'static str {
    match flow {
        "x-reply" => "What should the replies be about?",
        "ig-caption" => "What is the picture of?",
        _ => "What should the post be about?",
    }
}

impl Plugin for Social {
    fn init(&mut self, host: &Host, _info: &Value) {
        for flow in FLOWS {
            if let Err(why) = flow.checked() {
                host.log(format!("the workflow is wrong: {why}"));
            }
        }
        self.loading_flow = Some(host.storage_get(FLOW));
    }

    fn command(&mut self, host: &Host, _command: &str) {
        host.show_panel();
    }

    fn panel_open(&mut self, host: &Host) {
        self.draw(host);
    }

    fn ui_event(&mut self, host: &Host, event: UiEvent) {
        if let Some(id) = event.element.strip_prefix("flow.") {
            return self.pick(host, &id.to_string());
        }
        if agentos::handle(&mut self.machine, host, &event, &mut self.typed) {
            self.copied = false;
            self.keep(host);
            self.draw(host);
        }
    }

    fn pane_status(&mut self, host: &Host, status: PaneStatus) {
        self.machine.pane_status(host, &status);
        self.copy_caption(host);
        self.keep(host);
        self.draw(host);
    }

    fn answer(&mut self, host: &Host, id: u64, result: Result<Value, String>) {
        // Which workflow was last used, then the run that belongs to it.
        if self.loading_flow == Some(id) {
            self.loading_flow = None;
            if let Some(chosen) = result.as_ref().ok().and_then(|value| value["value"].as_str()) {
                self.machine = Machine::new(flow_by_id(chosen));
            }
            self.loading_run = Some(host.storage_get(RUN));
            return;
        }
        if self.loading_run == Some(id) {
            self.loading_run = None;
            if let Ok(value) = &result {
                self.machine.restore(&value["value"]);
                self.machine.resume(host);
            }
            return self.draw(host);
        }
        if self.machine.answer(host, id, &result) {
            self.copy_caption(host);
            self.keep(host);
            self.draw(host);
        }
    }
}

export_plugin!(Social);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_workflow_is_one_a_run_can_be_stopped_in() {
        for flow in FLOWS {
            flow.checked().unwrap_or_else(|why| panic!("{}: {why}", flow.id));
        }
    }

    #[test]
    fn every_step_that_opens_a_page_carries_the_rules_for_it() {
        // The browser rules are not advice: a step that drives a page without them is a step
        // that might sign in, install something, or act while it is meant to be reading.
        for flow in FLOWS {
            for step in flow.steps {
                let touches_pages = step.prompt.contains("{browser}");
                let names_a_site = step.prompt.contains("x.com") || step.prompt.contains("instagram.com");
                assert_eq!(touches_pages, names_a_site, "{}/{}", flow.id, step.id);
                if touches_pages {
                    assert!(flow.glossary.iter().any(|(name, _)| *name == "browser"), "{}/{}", flow.id, step.id);
                }
            }
        }
    }

    /// The rules are only rules if they reach the agent: a prompt that went out with the name in
    /// it, unfilled, is a step driving the user's signed-in browser with no rules at all.
    #[test]
    fn the_browser_rules_reach_the_agent_and_not_the_name_of_them() {
        let host = Host::new();
        let _ = agentty_plugin::host_stubs::taken();
        let mut machine = Machine::new(&X_POST);
        machine.start(&host, "a topic");
        let sent = agentty_plugin::host_stubs::taken();
        let text = sent[0]["params"]["text"].as_str().expect("a prompt was sent");
        assert!(text.contains("Never sign in"), "the rules did not reach it");
        assert!(text.contains("agentty browser navigate"), "nor did the commands");
        assert!(!text.contains("{browser}"), "the name went out instead of what it stands for");
        assert!(text.contains("a topic"), "nor did what the run is about");
    }

    #[test]
    fn a_step_that_only_reads_says_so() {
        for (flow, step) in [(&X_POST, "gather"), (&X_REPLY, "find"), (&IG_CAPTION, "research")] {
            let prompt = flow.steps.iter().find(|s| s.id == step).expect("the step is there").prompt;
            assert!(prompt.contains("Do not like"), "{step} does not say what reading means");
        }
    }

    #[test]
    fn a_post_that_is_too_long_or_is_not_a_post_is_sent_back() {
        assert!(fits_a_post("A short, finished thought about something.").is_ok());
        assert!(fits_a_post(&"a".repeat(POST_LIMIT + 1)).is_err());
        assert!(fits_a_post("").is_err());
        assert!(fits_a_post("TODO write this").is_err());
        assert!(fits_a_post("Here is the post:\n\n\"one\"\n\nand here is why\n1\n2\n3\n4").is_err());
    }

    #[test]
    fn the_agent_saying_what_it_is_about_to_do_is_not_the_answer() {
        // This is what a session read mid-work gives back, and taking it for the answer is how a
        // run walks its whole workflow in three seconds having found nothing.
        let mid_work = "I'll start by loading the Instagram home page in the in-app browser.\n\
            [tool: Bash agentty browser navigate https://www.instagram.com/]";
        assert!(is_an_answer(mid_work).is_err());
        let answer = format!("The account posts short captions, two or three lines, with the hashtags on a line of their own. {}", "The voice is plain and it does not use emoji. ".repeat(4));
        assert!(is_an_answer(&answer).is_ok());
    }

    #[test]
    fn a_page_that_asked_for_a_login_is_not_read_as_a_result() {
        for wall in ["Please sign in to see this", "It showed a captcha", "There were no posts"] {
            assert!(actually_looked(wall).is_err(), "{wall}");
        }
        let three = (1..=3)
            .map(|i| format!("@author{i} https://x.com/author{i}/status/{i} — a line about what this post actually says, at some length.\n"))
            .collect::<String>();
        assert!(lists_posts(&three).is_ok(), "{three}");
        assert!(lists_posts("@a https://x.com/a/1").is_err(), "one post is not enough to work from");
    }

    #[test]
    fn replies_are_bounded_in_number_and_in_length() {
        let five: String = (1..=MAX_REPLIES).map(|i| format!("https://x.com/a/{i}\nA short reply.\n")).collect();
        assert!(replies_that_fit(&five).is_ok());
        let six: String = (1..=MAX_REPLIES + 1).map(|i| format!("https://x.com/a/{i}\nA short reply.\n")).collect();
        assert!(replies_that_fit(&six).is_err(), "more than {MAX_REPLIES} replies is not sent");
        let long = format!("https://x.com/a/1\n{}", "a".repeat(POST_LIMIT + 1));
        assert!(replies_that_fit(&long).is_err());
        assert!(replies_that_fit("Just some text with no links").is_err());
    }

    #[test]
    fn a_caption_needs_its_hashtags_and_the_file_needs_its_name() {
        let caption = format!("A first line.\n\nThe rest of it.\n\n{}", "#tag ".repeat(8));
        assert!(has_a_caption(&caption).is_ok());
        assert!(has_a_caption("A caption with no tags at all").is_err());
        assert!(has_a_caption(&format!("x {}", "#tag ".repeat(20))).is_err());
        assert!(names_the_file("Saved to ./instagram-caption.txt").is_ok());
        assert!(names_the_file("Saved it.").is_err());
    }

    #[test]
    fn a_step_that_posts_has_to_say_what_it_posted() {
        assert!(links_to_what_it_posted("Posted: https://x.com/me/status/1").is_ok());
        assert!(links_to_what_it_posted("Done.").is_err(), "a run must not end on a claim with nothing behind it");
    }

    #[test]
    fn the_workflow_a_run_belongs_to_is_the_one_it_is_read_into() {
        assert_eq!(flow_by_id("x-reply").id, "x-reply");
        assert_eq!(flow_by_id("ig-caption").id, "ig-caption");
        // Something that is no longer offered falls back rather than starting a run of nothing.
        assert_eq!(flow_by_id("gone").id, X_POST.id);
    }
}
