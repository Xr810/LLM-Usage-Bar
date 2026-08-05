//! macOS-specific main-window reveal patch.
//!
//! The main window is built `visible: false` and only shown when the user picks
//! something from the tray. On that path the WKWebView negotiates its viewport
//! while the window is still hidden and does not renegotiate on `show()`, so the
//! page lays out against roughly half the window's size: a 1000x650 window
//! renders as if the viewport were 500x325, and everything appears at double
//! scale. Moving or resizing the window clears it, because that forces a real
//! bounds change.
//!
//! This is the same shape as failure mode B in [`crate::linux_fix`] — a surface
//! whose size was settled while hidden — so the remedy is the same: a 1px resize
//! and back, which is invisible but counts as a bounds change.
//!
//! Unlike the Linux path this needs no realize wait. AppKit applies `setFrame:`
//! synchronously on the main thread, so the two calls can go back to back on the
//! next tick without the compositor coalescing them away.

use tauri::{PhysicalSize, WebviewWindow};

/// Force the webview to renegotiate its viewport after the window is shown.
///
/// Call after `show()` on every path that reveals the main window from hidden.
pub(crate) fn renegotiate_webview_viewport(window: WebviewWindow) {
    tauri::async_runtime::spawn(async move {
        // Yield once so `show()` has been applied before the bounds change;
        // resizing in the same tick is coalesced with the show and does nothing.
        tokio::task::yield_now().await;

        match window.inner_size() {
            Ok(original) => {
                let bumped = PhysicalSize::new(original.width.saturating_add(1), original.height);
                if window.set_size(bumped).is_ok() {
                    let _ = window.set_size(original);
                }
            }
            Err(error) => {
                log::warn!("macOS: 读取主窗口尺寸失败，跳过视口重协商: {error}");
            }
        }
    });
}
