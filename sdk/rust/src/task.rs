//! `async` for plugins: write a flow that asks Agentty several things in a row the way it reads.
//!
//! ```ignore
//! agentty_plugin::task::spawn(async move {
//!     let page = task::call("browser/open", json!({ "url": "https://example.com" })).await?;
//!     task::sleep(2_000).await;
//!     let title = task::call("browser/eval", json!({ "tabId": page["tabId"], "script": "return document.title" })).await?;
//!     Ok::<_, String>(())
//! });
//! ```
//!
//! A module has one thread and runs only while it handles a message, so this is a small
//! executor of its own: tasks are polled when something they wait for arrives — an answer, or a
//! timer. Timers share one `host/timer` at a time (Agentty allows eight, of an hour at most), so
//! any number of tasks may sleep for any length.

use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

type Task = Pin<Box<dyn Future<Output = ()>>>;

thread_local! {
    static TASKS: RefCell<Vec<Task>> = const { RefCell::new(Vec::new()) };
    static SPAWNED: RefCell<Vec<Task>> = const { RefCell::new(Vec::new()) };
    static POLLING: Cell<bool> = const { Cell::new(false) };
    /// Calls a task waits on, and their answers once they are in.
    static WAITING: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
    static ANSWERS: RefCell<HashMap<u64, Result<Value, String>>> = RefCell::new(HashMap::new());
    /// Ids far from the ones `Host::call` hands out, so the two never meet.
    static NEXT_ID: Cell<u64> = const { Cell::new(1 << 40) };
    /// When each sleeping task wants to wake (ms), by a number of its own.
    static SLEEPS: RefCell<BTreeMap<u64, i64>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_SLEEP: Cell<u64> = const { Cell::new(1) };
    /// The `host/timer` calls in the air, and when the earliest of them fires.
    static TIMERS: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
    static ARMED_FOR: Cell<Option<i64>> = const { Cell::new(None) };
}

/// Shortest and longest wait Agentty takes.
const MIN_TIMER_MS: i64 = 100;
const MAX_TIMER_MS: i64 = 3_600_000;

/// Runs `future` alongside the plugin's other tasks.
pub fn spawn(future: impl Future<Output = ()> + 'static) {
    SPAWNED.with(|spawned| spawned.borrow_mut().push(Box::pin(future)));
    run();
}

/// Moves every task on as far as it can go. Agentty's messages call this; a plugin does not
/// need to.
pub fn run() {
    if POLLING.with(|p| p.replace(true)) {
        return;
    }
    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);
    loop {
        let mut tasks = TASKS.with(|t| std::mem::take(&mut *t.borrow_mut()));
        tasks.extend(SPAWNED.with(|s| std::mem::take(&mut *s.borrow_mut())));
        if tasks.is_empty() {
            break;
        }
        let mut kept = Vec::with_capacity(tasks.len());
        for mut task in tasks {
            if task.as_mut().poll(&mut context).is_pending() {
                kept.push(task);
            }
        }
        TASKS.with(|t| t.borrow_mut().extend(kept));
        // Tasks started while these were polled get their first turn now.
        if SPAWNED.with(|s| s.borrow().is_empty()) {
            break;
        }
    }
    POLLING.with(|p| p.set(false));
    arm();
}

/// Called with every answer Agentty sends: `true` when a task was waiting for it (or it was one
/// of the timers), which then runs on.
pub fn answered(id: u64, result: Result<Value, String>) -> bool {
    if TIMERS.with(|t| t.borrow_mut().remove(&id)) {
        ARMED_FOR.with(|a| a.set(None));
        run();
        return true;
    }
    if !WAITING.with(|w| w.borrow_mut().remove(&id)) {
        return false;
    }
    ANSWERS.with(|a| a.borrow_mut().insert(id, result));
    run();
    true
}

/// Any method of the protocol; resolves with its result or its error message.
pub fn call(method: &str, params: Value) -> impl Future<Output = Result<Value, String>> {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    WAITING.with(|w| w.borrow_mut().insert(id));
    crate::send_raw(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    Answer { id }
}

struct Answer {
    id: u64,
}

impl Future for Answer {
    type Output = Result<Value, String>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        match ANSWERS.with(|a| a.borrow_mut().remove(&self.id)) {
            Some(result) => Poll::Ready(result),
            None => Poll::Pending,
        }
    }
}

/// Resolves once `ms` have passed.
pub fn sleep(ms: u64) -> impl Future<Output = ()> {
    let due = crate::clock_ms() + ms as i64;
    let key = NEXT_SLEEP.with(|n| {
        let key = n.get();
        n.set(key + 1);
        key
    });
    SLEEPS.with(|s| s.borrow_mut().insert(key, due));
    Sleep { key, due }
}

struct Sleep {
    key: u64,
    due: i64,
}

impl Future for Sleep {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if crate::clock_ms() >= self.due {
            SLEEPS.with(|s| s.borrow_mut().remove(&self.key));
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        // A task still sleeping when the module goes is dropped after the table it is in.
        let _ = SLEEPS.try_with(|s| s.borrow_mut().remove(&self.key));
    }
}

/// Keeps one `host/timer` in the air for the earliest sleeper.
fn arm() {
    let Some(earliest) = SLEEPS.with(|s| s.borrow().values().min().copied()) else { return };
    if ARMED_FOR.with(|a| a.get()).is_some_and(|armed| armed <= earliest) {
        return;
    }
    let ms = (earliest - crate::clock_ms()).clamp(MIN_TIMER_MS, MAX_TIMER_MS);
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    TIMERS.with(|t| t.borrow_mut().insert(id));
    ARMED_FOR.with(|a| a.set(Some(crate::clock_ms() + ms)));
    crate::send_raw(&json!({ "jsonrpc": "2.0", "id": id, "method": "host/timer", "params": { "ms": ms } }));
}

fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(|_| RawWaker::new(std::ptr::null(), &VTABLE), |_| {}, |_| {}, |_| {});
    // SAFETY: every function of the table does nothing with the (null) data pointer.
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::host_stubs;
    use std::rc::Rc;

    #[test]
    fn a_task_goes_on_when_its_answer_arrives_and_when_its_time_comes() {
        host_stubs::set_clock(1_000);
        let _ = host_stubs::taken();
        let log = Rc::new(RefCell::new(Vec::new()));
        let seen = log.clone();
        spawn(async move {
            let value = call("storage/get", json!({ "key": "a" })).await;
            seen.borrow_mut().push(format!("got {value:?}"));
            sleep(5_000).await;
            seen.borrow_mut().push("slept".into());
        });
        let sent = host_stubs::taken();
        assert_eq!(sent[0]["method"], "storage/get");
        let id = sent[0]["id"].as_u64().unwrap();
        assert!(log.borrow().is_empty());
        assert!(answered(id, Ok(json!(1))));
        assert_eq!(log.borrow().as_slice(), ["got Ok(Number(1))"]);
        // One timer for the sleeper, for as long as it sleeps.
        let timers = host_stubs::taken();
        assert_eq!(timers.len(), 1);
        assert_eq!(timers[0]["method"], "host/timer");
        assert_eq!(timers[0]["params"]["ms"], 5_000);
        host_stubs::set_clock(6_000);
        assert!(answered(timers[0]["id"].as_u64().unwrap(), Ok(Value::Null)));
        assert_eq!(log.borrow().len(), 2);
        assert!(!answered(424_242, Ok(Value::Null)), "an answer nobody waits for is the plugin's own");
    }

    #[test]
    fn many_sleepers_share_one_timer() {
        host_stubs::set_clock(0);
        let _ = host_stubs::taken();
        let done = Rc::new(Cell::new(0));
        for ms in [3_000_u64, 1_000, 2_000, 7_200_000] {
            let done = done.clone();
            spawn(async move {
                sleep(ms).await;
                done.set(done.get() + 1);
            });
        }
        let timers: Vec<Value> = host_stubs::taken();
        // The first sleeper armed one for 3 s, then an earlier one for 1 s: no more than that.
        assert!(timers.len() <= 2, "{timers:?}");
        assert_eq!(timers.last().unwrap()["params"]["ms"], 1_000);
        host_stubs::set_clock(3_000);
        for timer in &timers {
            answered(timer["id"].as_u64().unwrap(), Ok(Value::Null));
        }
        assert_eq!(done.get(), 3);
        // The two-hour sleeper waits in hour-long pieces.
        let next = host_stubs::taken();
        assert_eq!(next.last().unwrap()["params"]["ms"], 3_600_000);
    }
}
