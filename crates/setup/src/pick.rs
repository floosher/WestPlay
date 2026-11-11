//! Pickers: a folder (the player showing WestPlay where the game is when it isn't found, an icon
//! set's folder), and files (wallpapers, an icon set's .zip).

use std::path::PathBuf;

#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog,
    IShellItem, SIGDN_FILESYSPATH,
};
#[cfg(windows)]
use windows::core::{HSTRING, PCWSTR};

/// Asks for a folder, over the window `owner` (its handle; 0 for none), titled `title`: the folder chosen, or
/// `None` if the player cancelled. Blocks until they've chosen; runs on a thread of its own (the dialog needs
/// COM, set up there as it wants).
#[cfg(windows)]
pub fn folder(owner: isize, title: &str) -> Option<PathBuf> {
    let title = title.to_owned();
    on_com_thread(move || {
        // SAFETY: on a thread with COM set up (`on_com_thread`); the owner window is the caller's, or none.
        unsafe { show(HWND(owner as *mut _), &title, None, false) }
    })
    .into_iter()
    .next()
}

/// Asks for a folder on Unix/Linux using desktop dialogs (zenity/kdialog).
#[cfg(unix)]
pub fn folder(_owner: isize, title: &str) -> Option<PathBuf> {
    if let Ok(output) = std::process::Command::new("zenity")
        .arg("--file-selection")
        .arg("--directory")
        .arg(format!("--title={title}"))
        .output()
    {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }
    if let Ok(output) = std::process::Command::new("kdialog")
        .arg("--getexistingdirectory")
        .arg(format!("--title={title}"))
        .output()
    {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }
    None
}

/// Asks for files, over the window `owner`, titled `title`: those of the kinds `extensions` (`["png",
/// "jpg"]`, shown as `kinds`), one or (`several`) more. None if the player cancelled.
#[cfg(windows)]
pub fn files(owner: isize, title: &str, kinds: &str, extensions: &[&str], several: bool) -> Vec<PathBuf> {
    let (title, kinds) = (title.to_owned(), kinds.to_owned());
    let pattern = extensions.iter().map(|extension| format!("*.{extension}")).collect::<Vec<_>>().join(";");
    on_com_thread(move || {
        // SAFETY: on a thread with COM set up (`on_com_thread`); the owner window is the caller's, or none.
        unsafe { show(HWND(owner as *mut _), &title, Some((&kinds, &pattern)), several) }
    })
}

/// Asks for files on Unix/Linux using desktop dialogs (zenity/kdialog).
#[cfg(unix)]
pub fn files(_owner: isize, title: &str, _kinds: &str, extensions: &[&str], several: bool) -> Vec<PathBuf> {
    let mut cmd = std::process::Command::new("zenity");
    cmd.arg("--file-selection").arg(format!("--title={title}"));
    if several {
        cmd.arg("--multiple").arg("--separator=\n");
    }
    if !extensions.is_empty() {
        let pattern = extensions.iter().map(|ext| format!("*.{ext}")).collect::<Vec<_>>().join(" ");
        cmd.arg(format!("--file-filter={pattern}"));
    }
    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let files: Vec<PathBuf> = text.lines().map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).collect();
            if !files.is_empty() {
                return files;
            }
        }
    }
    Vec::new()
}

/// Runs a dialog on a thread of its own, with COM set up as the dialog wants, and waits for it.
#[cfg(windows)]
fn on_com_thread(dialog: impl FnOnce() -> Vec<PathBuf> + Send + 'static) -> Vec<PathBuf> {
    std::thread::spawn(move || {
        // SAFETY: COM on this new thread, set up and torn down by it.
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        let chosen = dialog();
        if com {
            // SAFETY: undoes this thread's own CoInitializeEx, after its last COM call.
            unsafe { CoUninitialize() };
        }
        chosen
    })
    .join()
    .unwrap_or_default()
}

/// Shows Windows' picker: for a folder, or (`files`: its kinds' name and pattern) for files.
///
/// # Safety
///
/// COM must be set up on the calling thread, as single-threaded (`on_com_thread`).
#[cfg(windows)]
unsafe fn show(owner: HWND, title: &str, files: Option<(&str, &str)>, several: bool) -> Vec<PathBuf> {
    // SAFETY: the dialog is a COM object made here, on a thread with COM set up (the caller's promise); the
    // filter's strings live through the calls that read them; each result is read before it's freed.
    unsafe {
        let Ok(dialog) = CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) else {
            return Vec::new();
        };
        let Ok(options) = dialog.GetOptions() else { return Vec::new() };
        let options = match files {
            None => options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM,
            Some(_) if several => options | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM | FOS_ALLOWMULTISELECT,
            Some(_) => options | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM,
        };
        let _ = dialog.SetOptions(options);
        let _ = dialog.SetTitle(&HSTRING::from(title));
        if let Some((kinds, pattern)) = files {
            let (kinds, pattern) = (HSTRING::from(kinds), HSTRING::from(pattern));
            let filter = [COMDLG_FILTERSPEC { pszName: PCWSTR(kinds.as_ptr()), pszSpec: PCWSTR(pattern.as_ptr()) }];
            let _ = dialog.SetFileTypes(&filter);
        }
        // Cancelled: an error.
        if dialog.Show(Some(owner).filter(|owner| !owner.0.is_null())).is_err() {
            return Vec::new();
        }
        let Ok(results) = dialog.GetResults() else { return Vec::new() };
        let count = results.GetCount().unwrap_or(0);
        (0..count).filter_map(|at| path_of(&results.GetItemAt(at).ok()?)).collect()
    }
}

/// An item's path on disk.
///
/// # Safety
///
/// COM must be set up on the calling thread.
#[cfg(windows)]
unsafe fn path_of(item: &IShellItem) -> Option<PathBuf> {
    // SAFETY: the item is alive (borrowed); its name is a CoTaskMem string, read before it's freed.
    unsafe {
        let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let chosen = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.0 as *const _));
        chosen
    }
}
