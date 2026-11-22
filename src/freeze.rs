//! The freeze log: when the window stops responding for a few seconds, note what Develop was
//! doing, in `freeze.log` in the settings folder.
//!
//! The window's thread marks each step of a frame (`step`) and each wait for data another
//! thread may hold (`lock`). A watchdog thread looks every half second: once a frame has run
//! for `AFTER`, it writes the step it is on and the steps before it in that frame, with their
//! times, and again later if it is still stuck; when the window responds again it notes how long
//! the freeze lasted. Marking a step costs next to nothing, so it is always on.

use std::sync::{LockResult, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// How long a frame may run before it counts as a freeze.
const AFTER: Duration = Duration::from_secs(4);
/// Further notes while it stays frozen.
const AGAIN: [Duration; 3] = [Duration::from_secs(20), Duration::from_secs(60), Duration::from_secs(180)];
/// Steps kept per frame (the first ones, then the newest).
const TRAIL: usize = 64;

struct State {
    main: std::thread::ThreadId,
    frame: Option<Instant>,
    /// This frame's steps: when each began, and what it is.
    trail: Vec<(Instant, String)>,
    /// How many notes were written for this frame.
    noted: usize,
}

fn state() -> &'static Mutex<Option<State>> {
    static STATE: OnceLock<Mutex<Option<State>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

/// Start watching (call once, on the window's thread).
pub fn start() {
    if let Ok(mut s) = state().lock() {
        if s.is_some() {
            return;
        }
        *s = Some(State { main: std::thread::current().id(), frame: None, trail: Vec::new(), noted: 0 });
    }
    std::thread::Builder::new()
        .name("freeze watch".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_millis(500));
            let note = {
                let Ok(mut guard) = state().lock() else { return };
                let Some(s) = guard.as_mut() else { continue };
                let Some(began) = s.frame else { continue };
                let due = std::iter::once(AFTER).chain(AGAIN).nth(s.noted);
                match due {
                    Some(d) if began.elapsed() >= d => {
                        s.noted += 1;
                        Some(describe(s, began, s.noted > 1))
                    }
                    _ => None,
                }
            };
            if let Some(text) = note {
                write(&text);
            }
        })
        .ok();
}

/// A frame starts.
pub fn frame_begin() {
    with_main(|s| {
        s.frame = Some(Instant::now());
        s.trail.clear();
        s.noted = 0;
    });
}

/// The frame is done: if it was noted as a freeze, note that the window responds again.
pub fn frame_end() {
    let mut text = None;
    with_main(|s| {
        if let (Some(began), true) = (s.frame, s.noted > 0) {
            let last = s.trail.last().map_or("", |t| t.1.as_str()).to_owned();
            text = Some(format!("{}  responding again after {:.1} s (the last step was: {last})\n", now(), began.elapsed().as_secs_f32()));
        }
        s.frame = None;
    });
    if let Some(text) = text {
        write(&text);
    }
}

/// The window's thread starts on `what` (other threads are ignored).
pub fn step(what: impl Into<String>) {
    with_main(|s| {
        if s.frame.is_none() {
            return;
        }
        if s.trail.len() >= TRAIL {
            s.trail.remove(TRAIL / 2);
        }
        s.trail.push((Instant::now(), what.into()));
    });
}

/// Lock `m`, noting the wait as a step ("waiting for `what`") when on the window's thread, and
/// going back to the step it was on once it has the lock.
pub fn lock<'a, T>(m: &'a Mutex<T>, what: &str) -> LockResult<MutexGuard<'a, T>> {
    if let Ok(guard) = m.try_lock() {
        return Ok(guard);
    }
    let before = current();
    step(format!("waiting for {what} (another part of Develop has it)"));
    let result = m.lock();
    if let Some(before) = before {
        step(before);
    }
    result
}

fn current() -> Option<String> {
    let mut out = None;
    with_main(|s| out = s.trail.last().map(|t| t.1.clone()));
    out
}

fn with_main(f: impl FnOnce(&mut State)) {
    if let Ok(mut guard) = state().lock() {
        if let Some(s) = guard.as_mut().filter(|s| s.main == std::thread::current().id()) {
            f(s);
        }
    }
}

fn describe(s: &State, began: Instant, still: bool) -> String {
    let secs = began.elapsed().as_secs_f32();
    let doing = s.trail.last().map_or("(no step noted yet)", |t| t.1.as_str());
    let mut text = if still {
        format!("{}  still not responding after {secs:.0} s, on: {doing}\n", now())
    } else {
        format!(
            "{}  Develop {} has not responded for {secs:.0} s. It is on: {doing}\n  This frame so far:\n",
            now(),
            env!("CARGO_PKG_VERSION")
        )
    };
    if !still {
        for (i, (at, what)) in s.trail.iter().enumerate() {
            let took = s.trail.get(i + 1).map_or_else(|| at.elapsed(), |next| next.0 - *at);
            text.push_str(&format!("    {:>7.2} s  {what}  ({:.2} s)\n", (*at - began).as_secs_f32(), took.as_secs_f32()));
        }
    }
    text
}

fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970 to a calendar date (UTC), civil-from-days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rest / 3600, rest % 3600 / 60, rest % 60)
}

fn write(text: &str) {
    use std::io::Write;
    let Some(dir) = crate::settings::settings_dir() else { return };
    let file = dir.join("freeze.log");
    // Kept small: once big, the old part moves aside.
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::rename(&file, dir.join("freeze.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        let _ = f.write_all(text.as_bytes());
    }
    eprint!("{text}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        let s = super::now();
        assert!(s.starts_with("20") && s.ends_with("UTC"), "{s}");
    }
}
