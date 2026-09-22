//! Writing an AgentOS: a plugin that walks a piece of work through Agentty's agents.
//!
//! An AgentOS holds the skills and the rules for a trade — blogging, a marketing week, an
//! influencer's posting — and runs them through agents in sessions the user can read and take
//! over. The plugin itself does nothing: it has no files, no browser, no network beyond what it
//! asked for. The agent does the work, because the agent already has a terminal, the user's
//! tools, and Agentty's own browser (`agentty browser …`).
//!
//! What is here is the part every AgentOS has in common: a list of steps, a state machine that
//! sends one, waits for the agent to stop, reads what it wrote, checks it, and either goes on or
//! asks the user. Writing one is then a [`Workflow`] and a handful of prompts.
//!
//! ```ignore
//! use agentty_plugin::agentos::{Approval, Run, Step, Workflow};
//!
//! const BLOG: Workflow = Workflow {
//!     id: "blog",
//!     title: "Blog post",
//!     agent: Some("claude"),
//!     steps: &[
//!         Step { id: "outline", title: ["Outline", "", "", ""], prompt: OUTLINE, check: has_sections, approval: Approval::Auto },
//!         Step { id: "draft", title: ["Draft", "", "", ""], prompt: DRAFT, check: long_enough, approval: Approval::Ask },
//!     ],
//! };
//! ```
//!
//! Two rules an AgentOS is held to, and this module holds it to the first:
//!
//! - **A step that leaves the machine asks first.** Posting, pushing, paying, sending mail:
//!   [`Approval::Ask`], every time. [`Workflow::checked`] refuses a workflow whose last step is
//!   not one, because the last step is the one that publishes.
//! - **The user stays in front of it.** Every prompt goes into a session they can see, in a tab
//!   they can take over. An AgentOS that runs where nobody can watch is a plugin writing to the
//!   user's accounts while they are not looking.

use crate::{Host, PaneStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// How long to wait before looking again when a session has said nothing at all.
pub const LOOK_AGAIN_MS: u64 = 4_000;
/// How long a session has to stay stopped before what it wrote is read as the answer.
///
/// An agent between two tool calls is idle for a moment, and a session read in that moment gives
/// back what it happened to have said so far — half a sentence and the tool it was about to run.
/// A step that took that for its answer would go on to the next one on nothing.
pub const SETTLE_MS: u64 = 2_500;
/// How many times a step is sent back to the agent before the run stops and asks the user.
pub const MAX_TRIES: u32 = 3;
/// Turns of the session read when a step finishes.
pub const TURNS_READ: u64 = 40;
/// Characters of a step's output kept in the run — enough to show and to hand to the next step.
pub const MAX_OUTPUT: usize = 20_000;

/// What happens when a step's answer passes its check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// The next step starts on its own.
    Auto,
    /// The user reads what came back and presses Continue.
    Ask,
}

/// One step: what to ask an agent, and what must be true of the answer.
pub struct Step {
    pub id: &'static str,
    /// What the user sees this step called, in `[English, 한국어, 日本語, 中文]`. A language left
    /// empty reads English, so a workflow can be written in one language and gain the others.
    pub title: [&'static str; 4],
    /// The skill — what this step asks an agent to do. English, like every prompt that ships in
    /// code; the prompt itself tells the agent which language to write in.
    ///
    /// `{input}` is replaced by what the run was started with, and `{step.<id>}` by what an
    /// earlier step produced.
    pub prompt: &'static str,
    /// What must be true of what the agent wrote. `Err(what is missing)` is sent back to the
    /// same session, up to [`MAX_TRIES`] times.
    pub check: fn(&str) -> Result<(), String>,
    pub approval: Approval,
}

/// A trade, as steps.
pub struct Workflow {
    pub id: &'static str,
    /// What the session this workflow runs in is called. English: an agent reads it, and so does
    /// the plugin, to tell its own sessions apart from anyone else's.
    pub title: &'static str,
    /// What the user sees the workflow called, in `[English, 한국어, 日本語, 中文]`. Empty
    /// throughout means [`Workflow::title`].
    pub label: [&'static str; 4],
    /// `claude`, `codex`, or `None` for whatever Agentty starts by default.
    pub agent: Option<&'static str>,
    /// Pieces of prompt several steps share, as `(name, text)`. `{name}` in any step's prompt is
    /// replaced by `text` before it is sent — the rules for driving a browser, a house style, the
    /// shape an answer must have. Without it every step that needs them repeats them, and the one
    /// that forgets is the one that does something nobody wanted.
    pub glossary: &'static [(&'static str, &'static str)],
    pub steps: &'static [Step],
}

impl Step {
    /// What to call this step to the user.
    pub fn label(&self, lang: crate::text::Lang) -> &'static str {
        let label = crate::text::t(lang, self.title);
        if label.is_empty() {
            self.id
        } else {
            label
        }
    }
}

impl Workflow {
    /// Refuses a workflow that would publish without asking. Call it once, in `init`: a workflow
    /// is a constant, so this is a mistake in the plugin, not in what the user did.
    ///
    /// Two steps at least, because the user is asked before the last one runs and a workflow of
    /// one step has nowhere to put them; and a last step that asks, so a run ends by showing what
    /// it did rather than by going quiet.
    pub fn checked(&'static self) -> Result<&'static Self, String> {
        if self.steps.len() < 2 {
            return Err(format!("workflow \"{}\" needs at least two steps", self.id));
        }
        let mut seen: Vec<&str> = Vec::new();
        for step in self.steps {
            if seen.contains(&step.id) {
                return Err(format!("workflow \"{}\" has two steps called \"{}\"", self.id, step.id));
            }
            seen.push(step.id);
        }
        // The last step is the one that publishes, whatever it is called.
        if self.steps[self.steps.len() - 1].approval != Approval::Ask {
            return Err(format!("the last step of \"{}\" must ask before it finishes", self.id));
        }
        for (name, _) in self.glossary {
            if name.is_empty() || !self.steps.iter().any(|step| step.prompt.contains(&format!("{{{name}}}"))) {
                return Err(format!("nothing in \"{}\" uses {{{name}}}", self.id));
            }
        }
        Ok(self)
    }

    fn step(&self, index: usize) -> Option<&'static Step> {
        self.steps.get(index)
    }

    /// What to call this workflow to the user.
    pub fn label(&self, lang: crate::text::Lang) -> &'static str {
        let label = crate::text::t(lang, self.label);
        if label.is_empty() {
            self.title
        } else {
            label
        }
    }

    /// What a step's answer is followed by — with one Agentty insists on whatever the workflow
    /// says: the step before the last always asks.
    ///
    /// The last step is the one that acts on the world — posts, pushes, sends — and the answer
    /// the user is shown before it runs is what it will act on. A workflow that marked that step
    /// `Auto` would be a plugin writing to the user's accounts while they are not looking, and no
    /// workflow gets to choose that.
    fn approval(&self, index: usize) -> Approval {
        if index + 2 == self.steps.len() {
            return Approval::Ask;
        }
        self.steps.get(index).map_or(Approval::Ask, |step| step.approval)
    }
}

/// Where a run has got to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Nothing is running.
    #[default]
    Idle,
    /// The step's prompt is with an agent.
    Working,
    /// The agent has stopped, but not for long enough to be believed yet.
    Settling,
    /// The agent stopped; what it wrote is being read.
    Reading,
    /// The check failed and the agent was told what is missing.
    Fixing,
    /// Waiting for the user to look at what came back.
    Review,
    /// It cannot go on by itself: the agent is asking for something, the session is gone, or the
    /// step failed its check once too often.
    Stuck,
    Done,
}

impl State {
    /// Whether the run is waiting on an agent rather than on a person.
    pub fn is_busy(self) -> bool {
        matches!(self, State::Working | State::Settling | State::Reading | State::Fixing)
    }
}

/// A run, as it is kept in the plugin's storage: everything needed to pick it up again after a
/// restart, and nothing that would not survive one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    /// Which workflow this is, so a saved run is not read into a different one.
    pub workflow: String,
    /// What the run was started with — the topic, the brief, the account.
    pub input: String,
    pub step: usize,
    pub state: State,
    /// The session the steps run in.
    pub pane: Option<u64>,
    /// What each finished step produced, by step id.
    pub output: BTreeMap<String, String>,
    /// How often the current step has been sent back.
    pub tries: u32,
    /// Whether the session has been seen working since this step's prompt was sent. A pane is
    /// idle from the moment it opens until the agent picks the prompt up, and reading it before
    /// then gives back whatever was there — which is how a step finishes on nothing.
    #[serde(default)]
    pub taken_up: bool,
    /// Why the run is where it is, when there is a reason worth keeping: what a check found
    /// missing, what stopped it. English — it is what the plugin's log shows, and the panel puts
    /// the step's own name in front of it in the language the user reads. Empty when the state
    /// says everything.
    pub note: String,
    pub started_ms: i64,
}

/// What the run is waiting for an answer to. Not kept: a request id means nothing after a
/// restart, and [`Run::resume`] asks again instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Nothing,
    /// The prompt that starts a session; its answer carries the pane id.
    Prompt(u64),
    /// A read of the session.
    Session(u64),
    /// A look again later, because the session had nothing to say yet.
    Timer(u64),
    /// The stop is being given [`SETTLE_MS`] to prove it is a stop and not a pause.
    Settle(u64),
}

/// A run and what it is waiting for: the whole of an AgentOS's own state.
pub struct Machine {
    flow: &'static Workflow,
    run: Run,
    waiting: Waiting,
}

impl Machine {
    pub fn new(flow: &'static Workflow) -> Self {
        Self { flow, run: Run { workflow: flow.id.to_string(), ..Run::default() }, waiting: Waiting::Nothing }
    }

    pub fn run(&self) -> &Run {
        &self.run
    }

    pub fn flow(&self) -> &'static Workflow {
        self.flow
    }

    /// The step the run is on.
    pub fn step(&self) -> Option<&'static Step> {
        self.flow.step(self.run.step)
    }

    /// What the step the run is on produced, when it has.
    pub fn output(&self) -> Option<&str> {
        self.step().and_then(|step| self.run.output.get(step.id)).map(String::as_str)
    }

    /// Reads a run back from storage. A run saved by a different workflow, or one that ran off
    /// the end of this one's steps, is dropped rather than stepped into.
    pub fn restore(&mut self, value: &Value) {
        let Ok(run) = serde_json::from_value::<Run>(value.clone()) else { return };
        if run.workflow != self.flow.id || run.step > self.flow.steps.len() {
            return;
        }
        self.run = run;
        self.waiting = Waiting::Nothing;
    }

    /// The run, to keep under a storage key.
    pub fn saved(&self) -> Value {
        serde_json::to_value(&self.run).unwrap_or(Value::Null)
    }

    /// Starts a run. `input` is the topic, the brief, whatever the workflow is about.
    pub fn start(&mut self, host: &Host, input: impl Into<String>) {
        let input = input.into();
        self.run = Run {
            workflow: self.flow.id.to_string(),
            input,
            step: 0,
            state: State::Working,
            pane: None,
            output: BTreeMap::new(),
            tries: 0,
            taken_up: false,
            note: String::new(),
            started_ms: host.now_ms(),
        };
        self.send_step(host);
    }

    /// Picks a run up again after a restart: whatever it was waiting for is asked again.
    pub fn resume(&mut self, host: &Host) {
        match (self.run.state, self.run.pane) {
            // It was with an agent, and the answer — or the news that the pane is gone — comes
            // back through `pane/status`. Look at what it has meanwhile.
            (State::Working | State::Settling | State::Fixing | State::Reading, Some(_)) => {
                self.run.state = State::Reading;
                self.read_session(host);
            }
            // It was sent but never got as far as a pane id: nothing can be recovered.
            (State::Working | State::Settling | State::Fixing | State::Reading, None) => {
                self.stop(host, "Agentty restarted before the session started; start it again");
            }
            _ => {}
        }
    }

    /// The user has read what came back and wants the next step.
    pub fn approve(&mut self, host: &Host) {
        if self.run.state != State::Review {
            return;
        }
        self.advance(host);
    }

    /// Sends the current step again, from the top.
    pub fn retry(&mut self, host: &Host) {
        if self.run.state.is_busy() {
            return;
        }
        self.run.tries = 0;
        self.run.state = State::Working;
        self.run.note.clear();
        self.send_step(host);
    }

    /// Stops the run where it is. The session stays open: it is the user's.
    pub fn cancel(&mut self, host: &Host) {
        host.log(format!("{}: cancelled at step {}", self.flow.id, self.run.step));
        self.run.state = State::Idle;
        self.run.note.clear();
        self.waiting = Waiting::Nothing;
    }

    /// A pane this plugin started said something.
    pub fn pane_status(&mut self, host: &Host, status: &PaneStatus) {
        // A session the user placed themselves answers no prompt: the first status is where its
        // pane id arrives, and the title is what says it is ours.
        if self.run.pane.is_none() && self.run.state.is_busy() && status.title == self.flow.title {
            self.run.pane = Some(status.pane_id);
        }
        if self.run.pane != Some(status.pane_id) || !self.run.state.is_busy() {
            return;
        }
        if status.is_gone() {
            return self.stop(host, "the session was closed");
        }
        if status.needs_user() {
            return self.stop(host, "the agent is asking for something in its session");
        }
        // The agent has picked the prompt up. Until this, a pane is idle because it is new.
        if status.is_busy() {
            self.run.taken_up = true;
            // It went back to work: whatever it had said was not its answer after all.
            if self.run.state == State::Settling {
                self.run.state = State::Working;
                self.waiting = Waiting::Nothing;
            }
            return;
        }
        if status.is_done() && self.run.taken_up && matches!(self.run.state, State::Working | State::Fixing) {
            // Not read yet: an agent between two tool calls is idle for a moment.
            self.run.state = State::Settling;
            self.waiting = Waiting::Settle(host.wait(SETTLE_MS));
        }
    }

    /// An answer to something this machine asked for. `true` when it was one of its own.
    pub fn answer(&mut self, host: &Host, id: u64, result: &Result<Value, String>) -> bool {
        match self.waiting {
            Waiting::Prompt(waiting) if waiting == id => {
                self.waiting = Waiting::Nothing;
                match result {
                    // `target: "ask"` answers `{ status: "asked" }` with no pane: the user is
                    // picking where it goes, and the first `pane/status` brings the id.
                    Ok(value) => {
                        if let Some(pane) = value.get("paneId").and_then(Value::as_u64) {
                            self.run.pane = Some(pane);
                        }
                    }
                    Err(why) => self.stop(host, &format!("the prompt could not be sent: {why}")),
                }
                true
            }
            Waiting::Session(waiting) if waiting == id => {
                self.waiting = Waiting::Nothing;
                match result {
                    Ok(session) => self.read_answer(host, session),
                    // No session yet is not a failure: the agent may not have written anything.
                    Err(_) => self.look_again(host),
                }
                true
            }
            Waiting::Timer(waiting) if waiting == id => {
                self.waiting = Waiting::Nothing;
                if self.run.state == State::Reading {
                    self.read_session(host);
                }
                true
            }
            Waiting::Settle(waiting) if waiting == id => {
                self.waiting = Waiting::Nothing;
                // Still stopped: nothing put it back to work while this was waiting.
                if self.run.state == State::Settling {
                    self.run.state = State::Reading;
                    self.read_session(host);
                }
                true
            }
            _ => false,
        }
    }

    /// Sends the step the run is on, into its own session for the first step and into the same
    /// session for the rest — which is where the user can read the whole run in one place.
    fn send_step(&mut self, host: &Host) {
        let Some(step) = self.step() else { return self.finish(host) };
        let prompt = self.fill(step.prompt);
        let id = match self.run.pane {
            Some(pane) => host.prompt_pane(pane, prompt),
            None => host.start_session(self.flow.title, self.flow.agent, prompt),
        };
        self.waiting = Waiting::Prompt(id);
        self.run.state = State::Working;
        self.run.taken_up = false;
        self.run.note.clear();
        host.log(format!("{}: {} sent", self.flow.id, step.id));
    }

    fn read_session(&mut self, host: &Host) {
        let Some(pane) = self.run.pane else { return };
        self.waiting = Waiting::Session(host.session(pane, TURNS_READ));
    }

    fn look_again(&mut self, host: &Host) {
        self.waiting = Waiting::Timer(host.wait(LOOK_AGAIN_MS));
    }

    /// What came back from `session/get`.
    fn read_answer(&mut self, host: &Host, session: &Value) {
        let Some(step) = self.step() else { return };
        let text = last_written(session);
        if text.trim().is_empty() {
            return self.look_again(host);
        }
        match (step.check)(&text) {
            Ok(()) => {
                let mut text = text;
                if let Some((at, _)) = text.char_indices().nth(MAX_OUTPUT) {
                    text.truncate(at);
                }
                self.run.output.insert(step.id.to_string(), text);
                self.run.tries = 0;
                match self.flow.approval(self.run.step) {
                    Approval::Ask => {
                        self.run.state = State::Review;
                        self.run.note.clear();
                    }
                    Approval::Auto => self.advance(host),
                }
            }
            Err(why) => {
                self.run.tries += 1;
                if self.run.tries >= MAX_TRIES {
                    return self.stop(host, &format!("{why}, after {MAX_TRIES} tries"));
                }
                let Some(pane) = self.run.pane else { return self.stop(host, "the session is gone") };
                self.waiting = Waiting::Prompt(host.prompt_pane(pane, fix_prompt(&why)));
                self.run.state = State::Fixing;
                self.run.taken_up = false;
                self.run.note = why.clone();
                host.log(format!("{}: {} sent back ({why})", self.flow.id, step.id));
            }
        }
    }

    fn advance(&mut self, host: &Host) {
        self.run.step += 1;
        self.run.tries = 0;
        if self.step().is_none() {
            return self.finish(host);
        }
        self.send_step(host);
    }

    fn finish(&mut self, host: &Host) {
        self.run.state = State::Done;
        self.run.note.clear();
        self.waiting = Waiting::Nothing;
        host.log(format!("{}: done", self.flow.id));
    }

    fn stop(&mut self, host: &Host, why: &str) {
        self.run.state = State::Stuck;
        self.run.note = why.to_string();
        self.waiting = Waiting::Nothing;
        host.log(format!("{}: stopped — {why}", self.flow.id));
    }

    /// `{input}`, `{step.<id>}` and the workflow's own names, filled in with what the run has.
    fn fill(&self, prompt: &str) -> String {
        let mut out = prompt.replace("{input}", &self.run.input);
        for (id, text) in &self.run.output {
            out = out.replace(&format!("{{step.{id}}}"), text);
        }
        for (name, text) in self.flow.glossary {
            out = out.replace(&format!("{{{name}}}"), text);
        }
        out
    }
}

/// The ids of the buttons [`panel`] draws, so a plugin's `ui_event` can match on them.
pub mod button {
    /// Start a run with whatever is in [`INPUT`].
    pub const START: &str = "agentos.start";
    /// The field the run's input is typed into.
    pub const INPUT: &str = "agentos.input";
    /// Accept what the step produced and go on.
    pub const CONTINUE: &str = "agentos.continue";
    /// Ask the step again from the top.
    pub const RETRY: &str = "agentos.retry";
    /// Stop the run where it is.
    pub const CANCEL: &str = "agentos.cancel";
    /// Start again from the first step.
    pub const AGAIN: &str = "agentos.again";
}

/// Handles the buttons [`panel`] draws. `true` when the event was one of them, so a plugin can
/// mix its own buttons in and only see the rest.
pub fn handle(machine: &mut Machine, host: &Host, event: &crate::UiEvent, typed: &mut String) -> bool {
    match event.element.as_str() {
        button::INPUT => {
            *typed = event.text();
            true
        }
        button::START | button::AGAIN => {
            let input = if typed.trim().is_empty() { machine.run().input.clone() } else { typed.clone() };
            machine.start(host, input);
            true
        }
        button::CONTINUE => {
            machine.approve(host);
            true
        }
        button::RETRY => {
            machine.retry(host);
            true
        }
        button::CANCEL => {
            machine.cancel(host);
            true
        }
        _ => false,
    }
}

/// The run as a panel: the steps with the one it is on marked, what the last step produced, and
/// the buttons for whatever it is waiting for. `prompt` is what the input field asks for.
///
/// An AgentOS is free to draw its own instead — this is what it would have written.
pub fn panel(machine: &Machine, typed: &str, prompt: &str) -> Value {
    panel_in(machine, typed, prompt, crate::text::Lang::En)
}

/// [`panel`], in the language the user reads — `host.language()`. The words that are the panel's
/// own are translated; the workflow's title, its steps' titles and the reason a run stopped are
/// the plugin's, and are shown as it wrote them.
pub fn panel_in(machine: &Machine, typed: &str, prompt: &str, lang: crate::text::Lang) -> Value {
    use crate::text::t;
    use crate::ui;
    let run = machine.run();
    let mut children = vec![ui::styled_text(machine.flow().label(lang), "title")];

    let steps: Vec<Value> = machine
        .flow()
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let (mark, tone) = match (index.cmp(&run.step), run.state) {
                (std::cmp::Ordering::Less, _) => (t(lang, ["done", "완료", "完了", "完成"]), "success"),
                (std::cmp::Ordering::Equal, State::Done) => (t(lang, ["done", "완료", "完了", "完成"]), "success"),
                (std::cmp::Ordering::Equal, State::Stuck) => (t(lang, ["stopped", "멈춤", "停止", "已停止"]), "error"),
                (std::cmp::Ordering::Equal, State::Review) => (t(lang, ["to read", "확인 필요", "要確認", "待查看"]), "warning"),
                (std::cmp::Ordering::Equal, State::Idle) => (t(lang, ["next", "다음", "次", "下一步"]), "neutral"),
                (std::cmp::Ordering::Equal, _) => (t(lang, ["running", "진행 중", "実行中", "进行中"]), "info"),
                (std::cmp::Ordering::Greater, _) => ("", "neutral"),
            };
            let mut row = vec![ui::text(step.label(lang))];
            if !mark.is_empty() {
                row.push(ui::badge(mark, tone));
            }
            ui::row(row)
        })
        .collect();
    children.push(ui::section(t(lang, ["Steps", "단계", "ステップ", "步骤"]), steps));

    // One line saying where the run is: what the state means, and the reason when there is one.
    let step_now = machine.step().map(|step| step.label(lang));
    let said = match (run.state, step_now) {
        (State::Done, _) => t(lang, ["Done.", "완료했습니다.", "完了しました。", "已完成。"]).to_string(),
        (State::Review, Some(step)) => {
            format!("{step} — {}", t(lang, ["read it, then continue", "확인한 뒤 계속하세요", "確認してから続けてください", "查看后继续"]))
        }
        (State::Stuck, Some(step)) if !run.note.is_empty() => format!("{step} — {}", run.note),
        (State::Stuck, Some(step)) => format!("{step} — {}", t(lang, ["stopped", "멈췄습니다", "停止しました", "已停止"])),
        (State::Fixing, Some(step)) if !run.note.is_empty() => format!("{step} — {}", run.note),
        (State::Working | State::Settling | State::Reading | State::Fixing, Some(step)) => {
            format!("{step} — {}", t(lang, ["asked", "요청함", "依頼済み", "已请求"]))
        }
        _ => String::new(),
    };
    if !said.is_empty() {
        let style = match run.state {
            State::Stuck => "error",
            State::Done => "success",
            _ => "muted",
        };
        children.push(ui::styled_text(said, style));
    }
    if run.state.is_busy() {
        children.push(ui::spinner(t(lang, [
            "the agent is working — its session is open beside this",
            "에이전트가 작업 중입니다 — 옆에 세션이 열려 있습니다",
            "エージェントが作業中です — 隣にセッションが開いています",
            "智能体正在工作 — 会话就在旁边",
        ])));
    }

    // What the step that is waiting produced, so the user reads it before saying yes.
    if run.state == State::Review {
        if let Some(output) = machine.output() {
            children.push(ui::section(t(lang, ["What came back", "결과", "返ってきたもの", "返回的内容"]), vec![ui::styled_text(output, "body")]));
        }
    }

    children.push(ui::divider());
    let buttons = match run.state {
        State::Idle => vec![
            ui::input(button::INPUT, prompt, typed),
            ui::styled_button(button::START, t(lang, ["Start", "시작", "開始", "开始"]), "primary"),
        ],
        State::Review => vec![
            ui::styled_button(button::CONTINUE, t(lang, ["Continue", "계속", "続ける", "继续"]), "primary"),
            ui::styled_button(button::RETRY, t(lang, ["Ask again", "다시 요청", "もう一度依頼", "再问一次"]), "secondary"),
            ui::styled_button(button::CANCEL, t(lang, ["Stop", "중지", "停止", "停止"]), "ghost"),
        ],
        State::Stuck => vec![
            ui::styled_button(button::RETRY, t(lang, ["Try this step again", "이 단계 다시 시도", "このステップをやり直す", "重试该步骤"]), "primary"),
            ui::styled_button(button::CANCEL, t(lang, ["Stop", "중지", "停止", "停止"]), "ghost"),
        ],
        State::Done => vec![
            ui::input(button::INPUT, prompt, typed),
            ui::styled_button(button::AGAIN, t(lang, ["Run it again", "다시 실행", "もう一度実行", "再运行一次"]), "primary"),
        ],
        _ => vec![ui::styled_button(button::CANCEL, t(lang, ["Stop", "중지", "停止", "停止"]), "ghost")],
    };
    children.push(ui::row(buttons));
    ui::column(children)
}

/// What the agent wrote last, from a `session/get` answer.
pub fn last_written(session: &Value) -> String {
    session
        .get("turns")
        .and_then(Value::as_array)
        .and_then(|turns| {
            turns
                .iter()
                .rev()
                .find(|turn| turn.get("role").and_then(Value::as_str) == Some("assistant"))
                .and_then(|turn| turn.get("text").and_then(Value::as_str))
        })
        .unwrap_or_default()
        .to_string()
}

/// What a step that failed its check sends back. It names what is missing and asks for the whole
/// answer again, because half an answer is what got it here.
fn fix_prompt(why: &str) -> String {
    format!("That is not finished: {why}. Do it again and write the whole answer out, not only the part that changed.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_stubs;
    use serde_json::json;

    fn always_ok(_: &str) -> Result<(), String> {
        Ok(())
    }

    fn needs_the_word(text: &str) -> Result<(), String> {
        if text.contains("DONE") {
            Ok(())
        } else {
            Err("it does not say DONE".to_string())
        }
    }

    /// Three steps, so the middle one is the one the runner makes ask and the first is genuinely
    /// automatic — which is the shape every real workflow has.
    static FLOW: Workflow = Workflow {
        id: "test",
        title: "Test Flow",
        agent: Some("claude"),
        label: ["", "", "", ""],
        glossary: &[("rules", "be brief")],
        steps: &[
            Step { id: "one", title: ["One", "", "", ""], prompt: "do one with {input}, {rules}", check: always_ok, approval: Approval::Auto },
            Step { id: "two", title: ["Two", "", "", ""], prompt: "do two after {step.one}", check: needs_the_word, approval: Approval::Auto },
            Step { id: "three", title: ["Three", "", "", ""], prompt: "finish {step.two}", check: always_ok, approval: Approval::Ask },
        ],
    };

    static PUBLISHES_WITHOUT_ASKING: Workflow = Workflow {
        id: "bad",
        title: "Bad",
        agent: None,
        label: ["", "", "", ""],
        glossary: &[],
        steps: &[
            Step { id: "write", title: ["Write", "", "", ""], prompt: "write it", check: always_ok, approval: Approval::Auto },
            Step { id: "post", title: ["Post", "", "", ""], prompt: "post it", check: always_ok, approval: Approval::Auto },
        ],
    };

    static ONE_STEP: Workflow = Workflow {
        id: "one",
        title: "One",
        agent: None,
        label: ["", "", "", ""],
        glossary: &[],
        steps: &[Step { id: "post", title: ["Post", "", "", ""], prompt: "post it", check: always_ok, approval: Approval::Ask }],
    };

    /// Three steps, the middle one marked `Auto` although it is the one before the last.
    /// A glossary entry no prompt mentions: the sign of a `{name}` that was renamed in one place.
    static UNUSED_NAME: Workflow = Workflow {
        id: "unused",
        title: "Unused",
        agent: None,
        label: ["", "", "", ""],
        glossary: &[("rules", "…")],
        steps: &[
            Step { id: "a", title: ["A", "", "", ""], prompt: "no names here", check: always_ok, approval: Approval::Auto },
            Step { id: "b", title: ["B", "", "", ""], prompt: "nor here", check: always_ok, approval: Approval::Ask },
        ],
    };

    static SNEAKY: Workflow = Workflow {
        id: "sneaky",
        title: "Sneaky",
        agent: None,
        label: ["", "", "", ""],
        glossary: &[],
        steps: &[
            Step { id: "gather", title: ["Gather", "", "", ""], prompt: "gather", check: always_ok, approval: Approval::Auto },
            Step { id: "draft", title: ["Draft", "", "", ""], prompt: "draft", check: always_ok, approval: Approval::Auto },
            Step { id: "post", title: ["Post", "", "", ""], prompt: "post {step.draft}", check: always_ok, approval: Approval::Ask },
        ],
    };

    static TWO_STEPS_ONE_NAME: Workflow = Workflow {
        id: "same",
        title: "Same",
        agent: None,
        label: ["", "", "", ""],
        glossary: &[],
        steps: &[
            Step { id: "a", title: ["A", "", "", ""], prompt: "", check: always_ok, approval: Approval::Auto },
            Step { id: "a", title: ["A again", "", "", ""], prompt: "", check: always_ok, approval: Approval::Ask },
        ],
    };

    fn host() -> Host {
        let _ = host_stubs::taken();
        let _ = host_stubs::logged();
        Host::new()
    }

    /// The requests the plugin has made since the last call, as `(id, method, params)`.
    fn calls() -> Vec<(u64, String, Value)> {
        host_stubs::taken()
            .into_iter()
            .filter_map(|message| {
                let id = message.get("id")?.as_u64()?;
                let method = message.get("method")?.as_str()?.to_string();
                Some((id, method, message.get("params").cloned().unwrap_or(Value::Null)))
            })
            .collect()
    }

    /// The agent picks the prompt up, works, and stops — and the stop is given its settling time.
    /// Gives back what the machine did next, which is the read of the session.
    fn agent_stops(machine: &mut Machine, host: &Host, pane: u64) -> Vec<(u64, String, Value)> {
        machine.pane_status(host, &status(pane, "working"));
        machine.pane_status(host, &status(pane, "finished"));
        let settle = calls();
        assert_eq!(settle[0].1, "host/timer", "a stop is not believed straight away");
        machine.answer(host, settle[0].0, &Ok(json!({ "elapsedMs": SETTLE_MS })));
        let read = calls();
        assert_eq!(read[0].1, "session/get");
        read
    }

    fn session_saying(text: &str) -> Value {
        json!({ "turns": [{ "role": "user", "text": "go" }, { "role": "assistant", "text": text }] })
    }

    fn status(pane: u64, state: &str) -> PaneStatus {
        serde_json::from_value(json!({ "paneId": pane, "status": state, "running": state == "working", "title": "Test Flow" })).unwrap()
    }

    #[test]
    fn a_workflow_that_would_publish_without_asking_is_refused() {
        assert!(PUBLISHES_WITHOUT_ASKING.checked().is_err(), "the last step must ask");
        assert!(TWO_STEPS_ONE_NAME.checked().is_err(), "two steps cannot share an id");
        assert!(ONE_STEP.checked().is_err(), "one step has nowhere to ask the user");
        assert!(UNUSED_NAME.checked().is_err(), "a name no prompt uses would have gone out as it is");
        assert!(FLOW.checked().is_ok());
        assert!(SNEAKY.checked().is_ok());
    }

    #[test]
    fn the_user_is_asked_before_the_last_step_whatever_the_workflow_says() {
        // SNEAKY marks its middle step `Auto`, so the post would go out on what nobody read.
        let host = host();
        let mut machine = Machine::new(&SNEAKY);
        machine.start(&host, "x");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 1 })));
        let read = agent_stops(&mut machine, &host, 1);
        // Step one is genuinely automatic: it goes straight on.
        machine.answer(&host, read[0].0, &Ok(session_saying("gathered")));
        assert_eq!(machine.run().step, 1);
        let sent = calls();
        assert_eq!(sent[0].1, "prompt/inject");
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 1 })));

        // Step two says `Auto` and is overruled: it is what the post is made of.
        let read = agent_stops(&mut machine, &host, 1);
        machine.answer(&host, read[0].0, &Ok(session_saying("the post text")));
        assert_eq!(machine.run().state, State::Review, "the draft is shown before it is posted");
        assert!(calls().is_empty(), "nothing is sent until the user has read it");

        machine.approve(&host);
        assert_eq!(calls()[0].2["text"], "post the post text");
    }

    #[test]
    fn a_run_walks_its_steps_and_waits_for_the_user_at_the_end() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "a topic");

        // The first step opens a session of its own, named after the workflow, with the input in.
        let sent = calls();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, "prompt/inject");
        assert_eq!(sent[0].2["target"], "newTab");
        assert_eq!(sent[0].2["title"], "Test Flow");
        assert_eq!(sent[0].2["agent"], "claude");
        assert_eq!(sent[0].2["text"], "do one with a topic, be brief");
        assert_eq!(machine.run().state, State::Working);

        // Agentty answers with the pane the session is in.
        assert!(machine.answer(&host, sent[0].0, &Ok(json!({ "status": "sent", "paneId": 7 }))));
        assert_eq!(machine.run().pane, Some(7));

        // Nothing happens while the agent works.
        machine.pane_status(&host, &status(7, "working"));
        assert!(calls().is_empty());
        assert_eq!(machine.run().state, State::Working);

        // It stops: the session is read.
        let read = agent_stops(&mut machine, &host, 7);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].2["paneId"], 7);

        // What it wrote passes, and a step that needs no approval goes straight on — into the
        // same session, carrying what the step before it produced.
        assert!(machine.answer(&host, read[0].0, &Ok(session_saying("the outline"))));
        let next = calls();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].2["target"], "pane");
        assert_eq!(next[0].2["paneId"], 7);
        assert_eq!(next[0].2["text"], "do two after the outline");
        assert_eq!(machine.run().step, 1);

        // Step two is the one before the last, so its answer waits for the user however it is
        // marked — this is what the final step would act on.
        assert!(machine.answer(&host, next[0].0, &Ok(json!({ "status": "sent", "paneId": 7 }))));
        let read = agent_stops(&mut machine, &host, 7);
        assert!(machine.answer(&host, read[0].0, &Ok(session_saying("all DONE here"))));
        assert_eq!(machine.run().state, State::Review);
        assert!(calls().is_empty(), "nothing is sent until the user says so");

        machine.approve(&host);
        let last = calls();
        assert_eq!(last[0].2["text"], "finish all DONE here");
        assert!(machine.answer(&host, last[0].0, &Ok(json!({ "paneId": 7 }))));
        let read = agent_stops(&mut machine, &host, 7);
        assert!(machine.answer(&host, read[0].0, &Ok(session_saying("posted"))));
        assert_eq!(machine.run().state, State::Review);
        machine.approve(&host);
        assert_eq!(machine.run().state, State::Done);
        assert_eq!(machine.run().output["one"], "the outline");
        assert_eq!(machine.run().output["two"], "all DONE here");
        assert_eq!(machine.run().output["three"], "posted");
    }

    #[test]
    fn an_answer_that_is_not_finished_goes_back_to_the_agent_and_then_stops() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 3 })));
        let read = agent_stops(&mut machine, &host, 3);
        machine.answer(&host, read[0].0, &Ok(session_saying("outline")));
        // Now on step two, which wants the word.
        let _ = calls();

        for round in 1..MAX_TRIES {
            let read = agent_stops(&mut machine, &host, 3);
            machine.answer(&host, read[0].0, &Ok(session_saying("not yet")));
            let back = calls();
            assert_eq!(back.len(), 1, "round {round}");
            assert_eq!(back[0].1, "prompt/inject");
            assert!(back[0].2["text"].as_str().unwrap().contains("it does not say DONE"));
            assert_eq!(machine.run().state, State::Fixing);
            machine.answer(&host, back[0].0, &Ok(json!({ "paneId": 3 })));
        }

        // The last try: it stops and says why rather than asking for ever.
        let read = agent_stops(&mut machine, &host, 3);
        machine.answer(&host, read[0].0, &Ok(session_saying("still not")));
        assert_eq!(machine.run().state, State::Stuck);
        assert!(machine.run().note.contains("does not say DONE"), "{}", machine.run().note);
        assert!(machine.run().note.contains("after 3 tries"), "{}", machine.run().note);
        // The step it stopped on is named where the user reads it, in their language.
        let shown = panel_in(&machine, "", "", crate::text::Lang::Ko).to_string();
        assert!(shown.contains("Two"), "the step is named: {shown}");
        assert!(shown.contains("does not say DONE"), "and so is what it was missing");
        assert!(calls().is_empty(), "a run that gave up sends nothing more");
    }

    #[test]
    fn a_session_with_nothing_in_it_yet_is_looked_at_again_rather_than_failed() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 4 })));
        let read = agent_stops(&mut machine, &host, 4);

        // An agent that has written nothing yet, and a read that failed outright.
        machine.answer(&host, read[0].0, &Ok(json!({ "turns": [] })));
        let wait = calls();
        assert_eq!(wait[0].1, "host/timer");
        machine.answer(&host, wait[0].0, &Ok(json!({ "elapsedMs": LOOK_AGAIN_MS })));
        let read = calls();
        assert_eq!(read[0].1, "session/get");
        machine.answer(&host, read[0].0, &Err("no session yet".to_string()));
        assert_eq!(calls()[0].1, "host/timer");
        assert_eq!(machine.run().state, State::Reading, "it is still waiting, not stuck");
    }

    #[test]
    fn a_pane_that_is_idle_because_it_is_new_is_not_an_answer() {
        // A pane is idle from the moment it opens until the agent picks the prompt up. Reading it
        // then gives back whatever the session happened to hold — which is how a step finishes on
        // half a sentence and goes on to the next one.
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 6 })));
        machine.pane_status(&host, &status(6, "idle"));
        assert!(calls().is_empty(), "nothing is read before the agent has taken the prompt up");
        assert_eq!(machine.run().state, State::Working);

        // Once it has worked, idle means it has finished.
        machine.pane_status(&host, &status(6, "working"));
        machine.pane_status(&host, &status(6, "idle"));
        assert_eq!(calls()[0].1, "host/timer");
    }

    #[test]
    fn a_pause_between_two_tool_calls_does_not_end_the_step() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 8 })));
        machine.pane_status(&host, &status(8, "working"));

        // It stops for a moment between two tools.
        machine.pane_status(&host, &status(8, "idle"));
        let settle = calls();
        assert_eq!(settle[0].1, "host/timer");
        assert_eq!(machine.run().state, State::Settling);

        // ...and goes back to work before the wait is over.
        machine.pane_status(&host, &status(8, "working"));
        assert_eq!(machine.run().state, State::Working);
        machine.answer(&host, settle[0].0, &Ok(json!({ "elapsedMs": SETTLE_MS })));
        assert!(calls().is_empty(), "the session is not read on a pause the agent came back from");

        // The stop that lasts is the one that counts.
        let read = agent_stops(&mut machine, &host, 8);
        assert_eq!(read[0].1, "session/get");
    }

    #[test]
    fn a_run_stops_when_the_agent_asks_for_something_or_its_session_goes() {
        for (state, expected) in [("permission", "asking"), ("question", "asking"), ("closed", "closed"), ("exited", "closed")] {
            let host = host();
            let mut machine = Machine::new(&FLOW);
            machine.start(&host, "x");
            let sent = calls();
            machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 9 })));
            machine.pane_status(&host, &status(9, state));
            assert_eq!(machine.run().state, State::Stuck, "{state}");
            assert!(machine.run().note.contains(expected), "{state}: {}", machine.run().note);
        }
    }

    #[test]
    fn another_plugin_s_pane_is_not_this_run_s() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 1 })));
        machine.pane_status(&host, &status(2, "working"));
        machine.pane_status(&host, &status(2, "finished"));
        assert!(calls().is_empty(), "a pane that is not the run's is not read");
        assert_eq!(machine.run().state, State::Working);
    }

    #[test]
    fn a_session_the_user_placed_is_picked_up_from_its_first_status() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let sent = calls();
        // `target: "ask"` — the user is choosing where it goes, so there is no pane id yet.
        machine.answer(&host, sent[0].0, &Ok(json!({ "status": "asked" })));
        assert_eq!(machine.run().pane, None);
        machine.pane_status(&host, &status(12, "working"));
        assert_eq!(machine.run().pane, Some(12), "the session the user picked is the run's");
        assert_eq!(agent_stops(&mut machine, &host, 12)[0].1, "session/get");
    }

    #[test]
    fn a_run_is_kept_and_picked_up_again() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "the topic");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 5 })));
        let saved = machine.saved();

        // Agentty restarted: a new machine, the run read back from storage.
        let mut again = Machine::new(&FLOW);
        again.restore(&saved);
        assert_eq!(again.run().input, "the topic");
        assert_eq!(again.run().pane, Some(5));
        let _ = calls();
        again.resume(&host);
        assert_eq!(calls()[0].1, "session/get", "it asks again rather than waiting for an answer that will not come");

        // A run saved by a different workflow is not stepped into.
        let mut other = Machine::new(&PUBLISHES_WITHOUT_ASKING);
        other.restore(&saved);
        assert_eq!(other.run().step, 0);
        assert_eq!(other.run().state, State::Idle);
    }

    #[test]
    fn a_run_that_never_reached_a_session_says_so_instead_of_waiting() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let saved = machine.saved();
        let mut again = Machine::new(&FLOW);
        again.restore(&saved);
        again.resume(&host);
        assert_eq!(again.run().state, State::Stuck);
        assert!(again.run().note.contains("start it again"), "{}", again.run().note);
    }

    #[test]
    fn a_prompt_that_could_not_be_sent_stops_the_run() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Err("no window is open".to_string()));
        assert_eq!(machine.run().state, State::Stuck);
        assert!(machine.run().note.contains("no window is open"));
    }

    #[test]
    fn an_answer_to_something_else_is_left_alone() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let sent = calls();
        assert!(!machine.answer(&host, sent[0].0 + 100, &Ok(Value::Null)), "not the run's answer");
        assert!(machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 1 }))));
    }

    #[test]
    fn what_a_step_produced_reaches_the_prompts_after_it() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "the brief");
        let sent = calls();
        assert_eq!(sent[0].2["text"], "do one with the brief, be brief");
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 1 })));
        let read = agent_stops(&mut machine, &host, 1);
        machine.answer(&host, read[0].0, &Ok(session_saying("what one produced")));
        assert_eq!(calls()[0].2["text"], "do two after what one produced");
    }

    #[test]
    fn what_a_step_produced_is_kept_within_bounds() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        machine.start(&host, "x");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 1 })));
        let read = agent_stops(&mut machine, &host, 1);
        machine.answer(&host, read[0].0, &Ok(session_saying(&"a".repeat(MAX_OUTPUT * 2))));
        assert_eq!(machine.run().output["one"].chars().count(), MAX_OUTPUT);
    }

    /// Every id in a tree, so a test can say what the panel offers without drawing it.
    fn ids(tree: &Value) -> Vec<String> {
        let mut found = Vec::new();
        fn walk(node: &Value, found: &mut Vec<String>) {
            if let Some(id) = node.get("id").and_then(Value::as_str) {
                found.push(id.to_string());
            }
            for key in ["children", "items"] {
                if let Some(list) = node.get(key).and_then(Value::as_array) {
                    for child in list {
                        walk(child, found);
                    }
                }
            }
        }
        walk(tree, &mut found);
        found
    }

    #[test]
    fn the_panel_offers_what_the_run_is_waiting_for_and_nothing_else() {
        let host = host();
        let mut machine = Machine::new(&FLOW);

        // Nothing started: somewhere to type and a way to start.
        let tree = panel(&machine, "", "What about?");
        assert_eq!(ids(&tree), vec![button::INPUT, button::START]);

        machine.start(&host, "x");
        let sent = calls();
        machine.answer(&host, sent[0].0, &Ok(json!({ "paneId": 1 })));
        // While an agent works there is nothing to press but Stop.
        assert_eq!(ids(&panel(&machine, "", "")), vec![button::CANCEL]);

        // Waiting for the user: what came back is shown, with the three answers to it.
        let read = agent_stops(&mut machine, &host, 1);
        machine.answer(&host, read[0].0, &Ok(session_saying("the outline")));
        let _ = calls();
        let read = agent_stops(&mut machine, &host, 1);
        machine.answer(&host, read[0].0, &Ok(session_saying("DONE")));
        assert_eq!(machine.run().state, State::Review);
        let tree = panel(&machine, "", "");
        assert_eq!(ids(&tree), vec![button::CONTINUE, button::RETRY, button::CANCEL]);
        assert!(tree.to_string().contains("DONE"), "what came back is on screen before it is approved");
    }

    #[test]
    fn the_panel_is_in_the_language_the_user_reads() {
        use crate::text::Lang;
        let host = host();
        let mut machine = Machine::new(&FLOW);

        let korean = panel_in(&machine, "", "무엇에 대해?", Lang::Ko).to_string();
        assert!(korean.contains("시작"), "the button is not translated");
        assert!(korean.contains("단계"), "nor the heading");
        assert!(korean.contains("무엇에 대해?"), "what the plugin wrote is left as it wrote it");
        assert!(korean.contains("Test Flow"), "and so is the workflow's own title");

        // Every language draws, and the default is English.
        for lang in [Lang::En, Lang::Ko, Lang::Ja, Lang::Zh] {
            let tree = panel_in(&machine, "", "", lang);
            assert_eq!(ids(&tree), vec![button::INPUT, button::START], "{lang:?}");
        }
        assert_eq!(panel(&machine, "", ""), panel_in(&machine, "", "", Lang::En));

        // And once a run is going, so are the words that only appear then.
        machine.start(&host, "x");
        let _ = calls();
        let japanese = panel_in(&machine, "", "", Lang::Ja).to_string();
        assert!(japanese.contains("停止"), "the only button there is");
        assert!(japanese.contains("エージェントが作業中"), "nor the line under it");
    }

    #[test]
    fn the_panel_s_buttons_do_what_they_say() {
        let host = host();
        let mut machine = Machine::new(&FLOW);
        let mut typed = String::new();

        let event = |id: &str, value: Option<&str>| -> crate::UiEvent {
            serde_json::from_value(json!({ "element": id, "event": "click", "value": value })).unwrap()
        };

        assert!(handle(&mut machine, &host, &event(button::INPUT, Some("a topic")), &mut typed));
        assert_eq!(typed, "a topic");
        assert!(handle(&mut machine, &host, &event(button::START, None), &mut typed));
        assert_eq!(machine.run().input, "a topic");
        assert_eq!(calls()[0].2["text"], "do one with a topic, be brief");

        // A button the AgentOS added itself is left to it.
        assert!(!handle(&mut machine, &host, &event("mine", None), &mut typed));

        assert!(handle(&mut machine, &host, &event(button::CANCEL, None), &mut typed));
        assert_eq!(machine.run().state, State::Idle);
    }

    #[test]
    fn the_last_thing_the_agent_wrote_is_what_is_read() {
        let session = json!({ "turns": [
            { "role": "assistant", "text": "first" },
            { "role": "user", "text": "and then?" },
            { "role": "assistant", "text": "second" },
        ] });
        assert_eq!(last_written(&session), "second");
        assert_eq!(last_written(&json!({})), "");
        assert_eq!(last_written(&json!({ "turns": [{ "role": "user", "text": "only me" }] })), "");
    }
}
