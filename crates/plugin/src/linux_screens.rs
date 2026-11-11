//! Native Linux screen management.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::bridge;
use crate::log;

static ON_SUPPORTED_TRUCK: AtomicBool = AtomicBool::new(true);

/// Whether the truck's screen is one WestPlay shows on.
pub fn on_supported_truck() -> bool {
    ON_SUPPORTED_TRUCK.load(Ordering::Acquire)
}

/// The player is in another truck (or the game told its truck again).
pub fn truck_changed(id: &str) {
    if let Some(bridge) = bridge::get() {
        bridge.plugin.truck_supported.store(1, Ordering::Release);
    }
    ON_SUPPORTED_TRUCK.store(true, Ordering::Release);
    if !id.is_empty() {
        log!("screen: truck {id} active on native Linux");
    }
}

/// Called once per presented game frame.
pub fn on_frame(_frame: u64) {}

/// Logs screen texture statistics.
pub fn log_stats() {}
