//! Native Linux graphics backend and frame clock stubs.

use crate::log;

/// In the game's process, before hooks are enabled.
///
/// # Safety
/// Called during plugin initialization.
pub unsafe fn install() -> Result<(), String> {
    log!("linux graphics backend initialized");
    Ok(())
}

/// Called during plugin shutdown.
pub fn on_unload() {
    log!("linux graphics backend unloaded");
}

/// Whether the game is currently showing/active.
pub fn game_shown() -> bool {
    true
}
