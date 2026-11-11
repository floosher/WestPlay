//! Startup timing: each stage of a launch logged once (`log.rs`: the console in development, and the app's
//! log), with the local time and how long since the process started, to see where a slow launch goes. Start
//! `npm run dev` with the time printed (PowerShell: `Get-Date -Format HH:mm:ss.fff; npm run dev`), and the
//! gap to the first line is the dev server and the build before the app.

use std::collections::HashSet;
use std::sync::Mutex;
#[cfg(unix)]
use std::sync::OnceLock;
#[cfg(unix)]
use std::time::Instant;

#[cfg(windows)]
use windows::Win32::Foundation::FILETIME;
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTimeAsFileTime};
#[cfg(windows)]
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
#[cfg(unix)]
static STARTED: OnceLock<Instant> = OnceLock::new();

#[cfg(windows)]
fn now_time() -> (u16, u16, u16, u16) {
    // SAFETY: a plain query of the time.
    let now = unsafe { GetLocalTime() };
    (now.wHour, now.wMinute, now.wSecond, now.wMilliseconds)
}

#[cfg(unix)]
fn now_time() -> (u16, u16, u16, u16) {
    let mut tv = libc::timeval { tv_sec: 0, tv_usec: 0 };
    // SAFETY: pointers to locals.
    unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()) };
    let mut tm = libc::tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null_mut(),
    };
    // SAFETY: pointers to locals.
    unsafe { libc::localtime_r(&tv.tv_sec, &mut tm) };
    (tm.tm_hour as u16, tm.tm_min as u16, tm.tm_sec as u16, (tv.tv_usec / 1000) as u16)
}

/// Prints `stage` with the time, the first time it's reached (later calls with the same stage do nothing).
pub fn mark(stage: &str) {
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.get_or_insert_with(HashSet::new).insert(stage.to_owned()) {
        return;
    }
    let (hour, minute, second, millis) = now_time();
    let uptime = uptime_ms().map_or_else(|| "?".to_owned(), |ms| ms.to_string());
    log!(
        "[startup] {:02}:{:02}:{:02}.{:03}  +{uptime:>5} ms  {stage}",
        hour,
        minute,
        second,
        millis
    );
}

/// Milliseconds since this process was created.
#[cfg(windows)]
fn uptime_ms() -> Option<u64> {
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    // SAFETY: this process's own times, into locals.
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
    // SAFETY: a plain query of the time.
    let now = unsafe { GetSystemTimeAsFileTime() };
    // FILETIMEs count 100 ns steps.
    let ticks = |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Some(ticks(now).saturating_sub(ticks(created)) / 10_000)
}

#[cfg(unix)]
fn uptime_ms() -> Option<u64> {
    let started = STARTED.get_or_init(Instant::now);
    Some(started.elapsed().as_millis() as u64)
}
