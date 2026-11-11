//! WestPlay's map builder (`westplay-maps.exe`): builds the one map WestPlay asks for, in a process of its own
//! (westplay_setup::map_builder). The request is a JSON line on the standard input; how it goes, a JSON line each
//! on the standard output; anything that ends it unasked (a panic, a native crash, memory that couldn't be had) on
//! the standard error, which WestPlay keeps in its log.
//!
//! Its memory allocator is chosen as it starts, before anything is allocated ([`Chosen`]): mimalloc when WestPlay
//! asks for a faster build (`WESTPLAY_MAP_ALLOCATOR=mimalloc`), Windows' own otherwise. A process keeps the one it
//! started with: memory one allocator gave is only ever handed back to it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU8, Ordering};

use westplay_setup::map_builder::{self, ALLOCATOR_VARIABLE, MIMALLOC, Request, Said};
#[cfg(windows)]
use westplay_shared::crash;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, WriteFile};
#[cfg(windows)]
use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
#[cfg(windows)]
use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
#[cfg(windows)]
use windows::Win32::System::Environment::GetEnvironmentVariableW;
#[cfg(windows)]
use windows::core::PCWSTR;

#[global_allocator]
static ALLOCATOR: Chosen = Chosen;

/// The allocator this process was started with: 0 not chosen yet, 1 Windows', 2 mimalloc.
static CHOSEN: AtomicU8 = AtomicU8::new(0);

/// Windows' allocator or mimalloc, as the process was started ([`ALLOCATOR_VARIABLE`]), the same for its whole life.
struct Chosen;

impl Chosen {
    fn mimalloc() -> bool {
        let chosen = match CHOSEN.load(Ordering::Acquire) {
            0 => {
                let chosen = if variable_is(ALLOCATOR_VARIABLE, MIMALLOC) { 2 } else { 1 };
                CHOSEN.store(chosen, Ordering::Release);
                chosen
            }
            chosen => chosen,
        };
        chosen == 2
    }
}

/// Whether the environment variable `name` is `value`, read with no memory allocated (the allocator isn't chosen
/// yet when it's read).
#[cfg(windows)]
fn variable_is(name: &str, value: &str) -> bool {
    let mut wide_name = [0u16; 64];
    for (at, unit) in name.encode_utf16().take(63).enumerate() {
        wide_name[at] = unit;
    }
    let mut found = [0u16; 64];
    // SAFETY: `wide_name` is nul-terminated (one unit at least left 0), and `found` a local the call writes into.
    let length = unsafe { GetEnvironmentVariableW(PCWSTR(wide_name.as_ptr()), Some(&mut found)) } as usize;
    length < found.len() && found[..length].iter().copied().eq(value.encode_utf16())
}

#[cfg(unix)]
fn variable_is(name: &str, value: &str) -> bool {
    let mut c_name = [0u8; 64];
    if name.len() >= c_name.len() {
        return false;
    }
    c_name[..name.len()].copy_from_slice(name.as_bytes());
    // SAFETY: c_name is null-terminated and valid.
    let val_ptr = unsafe { libc::getenv(c_name.as_ptr() as *const libc::c_char) };
    if val_ptr.is_null() {
        return false;
    }
    // SAFETY: val_ptr is null-terminated C string from getenv.
    let c_str = unsafe { std::ffi::CStr::from_ptr(val_ptr) };
    c_str.to_bytes() == value.as_bytes()
}

// SAFETY: every call goes to the one allocator chosen before the first allocation and kept after, so memory is only
// ever handed back to the allocator that gave it; each is a sound GlobalAlloc, called with the caller's arguments.
unsafe impl GlobalAlloc for Chosen {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as GlobalAlloc's contract, passed on.
        unsafe { if Self::mimalloc() { mimalloc::MiMalloc.alloc(layout) } else { System.alloc(layout) } }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as GlobalAlloc's contract, passed on.
        unsafe { if Self::mimalloc() { mimalloc::MiMalloc.alloc_zeroed(layout) } else { System.alloc_zeroed(layout) } }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as GlobalAlloc's contract, passed on to the allocator that gave `ptr`.
        unsafe { if Self::mimalloc() { mimalloc::MiMalloc.dealloc(ptr, layout) } else { System.dealloc(ptr, layout) } }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as GlobalAlloc's contract, passed on to the allocator that gave `ptr`.
        unsafe {
            if Self::mimalloc() {
                mimalloc::MiMalloc.realloc(ptr, layout, new_size)
            } else {
                System.realloc(ptr, layout, new_size)
            }
        }
    }
}

fn main() {
    catch_crashes();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        fail("no request on the standard input");
    }
    let request: Request = serde_json::from_str(&line).unwrap_or_else(|error| fail(&format!("not a request: {error}")));
    // Faster: every thread, more drawn at once while the memory has room (as WestPlay's own builds did).
    let _effort = request.fast.then(|| {
        let total = total_memory().unwrap_or(0);
        westplay_mapdata::threads::Effort::full(westplay_mapdata::threads::room_on(total, memory))
    });
    let mut out = std::io::stdout().lock();
    let mut say = |said: Said| {
        let _ = serde_json::to_writer(&mut out, &said);
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    };
    let ended = map_builder::build(&request, &mut say, &memory);
    say(ended);
}

/// Ends the process, saying why on the standard error.
fn fail(why: &str) -> ! {
    eprintln!("westplay-maps: {why}");
    std::process::exit(2)
}

/// This process's memory, its private bytes (bytes).
#[cfg(windows)]
fn memory() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut counters =
        PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    // SAFETY: `counters` is a local with its size set, read as the larger form the size says it is.
    unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(), counters.cb)
    }
    .ok()?;
    Some(counters.PrivateUsage as u64)
}

#[cfg(unix)]
fn memory() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let mut fields = statm.split_whitespace();
    let _size = fields.next()?;
    let resident: u64 = fields.next()?.parse().ok()?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size > 0 {
        Some(resident * page_size as u64)
    } else {
        Some(resident * 4096)
    }
}

/// The PC's memory in all (bytes).
#[cfg(windows)]
fn total_memory() -> Option<u64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `status` is a local with its size set, as the call requires.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(status.ullTotalPhys)
}

#[cfg(unix)]
fn total_memory() -> Option<u64> {
    let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages > 0 && page_size > 0 {
        Some(pages as u64 * page_size as u64)
    } else {
        None
    }
}

/// A native crash said on the standard error (WestPlay keeps it in its log): what it was and whose code it was in.
#[cfg(windows)]
fn catch_crashes() {
    // SAFETY: `on_crash` is this executable's own, valid for the process's life.
    unsafe {
        SetUnhandledExceptionFilter(Some(on_crash));
    }
}

#[cfg(unix)]
fn catch_crashes() {}

/// # Safety
/// Called by Windows with the exception's pointers, as the process is failing: no locks, no memory taken.
#[cfg(windows)]
unsafe extern "system" fn on_crash(info: *const EXCEPTION_POINTERS) -> i32 {
    /// Windows' `EXCEPTION_CONTINUE_SEARCH`: its own handling goes on (the process ends).
    const CONTINUE_SEARCH: i32 = 0;
    // SAFETY: Windows' pointers to the exception, only read; the standard error, written once.
    unsafe {
        if let Some(record) = info.as_ref().and_then(|info| info.ExceptionRecord.as_ref())
            && let Ok(stderr) = GetStdHandle(STD_ERROR_HANDLE)
        {
            let mut line = crash::Line::new();
            let _ = line.write_str("westplay-maps crashed: ");
            crash::describe(record, &mut line);
            let _ = line.write_str("\n");
            let _ = WriteFile(stderr, Some(line.as_bytes()), None, None);
            let _ = FlushFileBuffers(stderr);
        }
    }
    CONTINUE_SEARCH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_variable_without_memory() {
        // SAFETY: the test's own process, its variables set before anything reads them here.
        unsafe { std::env::set_var("WESTPLAY_TEST_VARIABLE", "mimalloc") };
        assert!(variable_is("WESTPLAY_TEST_VARIABLE", "mimalloc"));
        assert!(!variable_is("WESTPLAY_TEST_VARIABLE", "mimallo"));
        assert!(!variable_is("WESTPLAY_TEST_MISSING", "mimalloc"));
    }
}
