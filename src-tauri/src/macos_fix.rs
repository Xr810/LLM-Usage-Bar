//! macOS-specific main-window reveal patch.
//!
//! The window opens at roughly a third of its area and snaps back to full size
//! the moment the user nudges it. The cause is a unit mismatch that has become
//! self-sustaining in the saved window state:
//!
//! `tauri-plugin-window-state` measures with `inner_size()` and restores with
//! `set_size(PhysicalSize)` — both in *physical pixels*, which round-trips
//! correctly only if the scale factor is the same at save and at restore. The
//! main window is built `visible: false`, and a window that has never been
//! placed on a screen reports a backing scale factor of 1, so the first save
//! records logical points as if they were pixels: 1000x650.
//!
//! Restoring 1000x650 *pixels* onto a 2x display gives a 500x325 point window.
//! The next save measures that window — 500x325 points at 2x is 1000x650 pixels
//! — and writes the same wrong number back. The state file has reached a fixed
//! point, which is why the bug survives every restart.
//!
//! The configured minimum is what breaks the loop. AppKit does not clamp a
//! programmatic `setFrame:` to `minSize`, but it does clamp on the first
//! constraint pass — which is exactly the "nudge it and it jumps" symptom. So a
//! window below its own minimum is never a size the user chose; it is a
//! measurement artifact, and the configured default is restored over it. The
//! next save then records a correct size and the state file repairs itself.
//!
//! (An earlier version of this module blamed the webview's viewport and did a
//! 1px resize after `show()`. That was wrong — the text renders at its normal
//! size, only the window is small — and the nudge is gone.)

use tauri::{LogicalSize, Manager, WebviewWindow};

/// Size to restore, or `None` when the current size is one the user could have
/// chosen and must be left alone.
fn corrected_size(
    current: LogicalSize<f64>,
    minimum: LogicalSize<f64>,
    default: LogicalSize<f64>,
) -> Option<LogicalSize<f64>> {
    // Half a point of slack: a frame is rounded to the backing grid, so a
    // window sitting exactly on its minimum can measure a hair under it.
    let undersized = current.width + 0.5 < minimum.width || current.height + 0.5 < minimum.height;
    undersized.then_some(default)
}

/// Undo a window size restored from state that was recorded at the wrong scale.
///
/// Call after `show()` on every path that reveals the main window from hidden:
/// the scale factor is only trustworthy once the window is on a screen.
pub(crate) fn repair_undersized_window(window: WebviewWindow) {
    tauri::async_runtime::spawn(async move {
        // Yield once so `show()` has been applied; a window measured in the
        // same tick can still report the size it had while hidden.
        tokio::task::yield_now().await;

        let Some(config) = window
            .config()
            .app
            .windows
            .iter()
            .find(|candidate| candidate.label == window.label())
            .cloned()
        else {
            return;
        };
        // With no configured minimum there is no way to tell a bad size from a
        // small one, so nothing is touched.
        let (Some(min_width), Some(min_height)) = (config.min_width, config.min_height) else {
            return;
        };

        let (size, scale) = match (window.inner_size(), window.scale_factor()) {
            (Ok(size), Ok(scale)) => (size, scale),
            _ => {
                log::warn!("macOS: 读取主窗口尺寸失败，跳过尺寸修复");
                return;
            }
        };

        let current: LogicalSize<f64> = size.to_logical(scale);
        let Some(corrected) = corrected_size(
            current,
            LogicalSize::new(min_width, min_height),
            LogicalSize::new(config.width, config.height),
        ) else {
            return;
        };

        log::info!(
            "macOS: 主窗口恢复尺寸 {}x{} 小于配置下限 {min_width}x{min_height}，重置为 {}x{}",
            current.width,
            current.height,
            corrected.width,
            corrected.height,
        );
        if let Err(error) = window.set_size(corrected) {
            log::warn!("macOS: 重置主窗口尺寸失败: {error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: LogicalSize<f64> = LogicalSize {
        width: 900.0,
        height: 600.0,
    };
    const DEFAULT: LogicalSize<f64> = LogicalSize {
        width: 1000.0,
        height: 650.0,
    };

    #[test]
    fn restores_the_default_when_state_was_recorded_at_the_wrong_scale() {
        // 1000x650 pixels restored onto a 2x display.
        assert_eq!(
            corrected_size(LogicalSize::new(500.0, 325.0), MIN, DEFAULT),
            Some(DEFAULT)
        );
    }

    #[test]
    fn leaves_a_size_the_user_could_have_dragged_to() {
        assert_eq!(
            corrected_size(LogicalSize::new(900.0, 600.0), MIN, DEFAULT),
            None
        );
        assert_eq!(
            corrected_size(LogicalSize::new(1400.0, 900.0), MIN, DEFAULT),
            None
        );
    }

    #[test]
    fn tolerates_a_frame_rounded_a_hair_under_the_minimum() {
        // A window sitting on its minimum must not be resized on every reveal.
        assert_eq!(
            corrected_size(LogicalSize::new(899.7, 599.8), MIN, DEFAULT),
            None
        );
    }

    #[test]
    fn a_single_undersized_edge_is_enough() {
        assert_eq!(
            corrected_size(LogicalSize::new(1000.0, 325.0), MIN, DEFAULT),
            Some(DEFAULT)
        );
    }
}
