use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(any(target_os = "macos", test))]
use tauri::tray::{MouseButton, MouseButtonState};
use tauri::{AppHandle, Emitter, Manager};
#[cfg(any(target_os = "macos", test))]
use tauri::{PhysicalPosition, PhysicalRect, PhysicalSize};
#[cfg(target_os = "macos")]
use tauri::{Rect, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::error::AppError;

pub const TRAY_POPOVER_LABEL: &str = "tray-popover";
#[cfg(target_os = "macos")]
const POPOVER_WIDTH: f64 = 380.0;
#[cfg(target_os = "macos")]
const POPOVER_HEIGHT: f64 = 520.0;
#[cfg(any(target_os = "macos", test))]
const POPOVER_GAP_PHYSICAL: i32 = 8;

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayClickAction {
    TogglePopover,
    HidePopover,
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MainWindowDestination {
    Usage { agent_module_id: Option<String> },
    ProviderBudget { provider_id: Option<String> },
}

#[derive(Debug)]
struct PendingMainDestinationAttempt {
    marker: Arc<()>,
}

#[derive(Debug)]
struct PendingMainDestinationEntry {
    marker: Arc<()>,
    destination: MainWindowDestination,
}

#[derive(Debug, Default)]
struct PendingMainDestinationSlot {
    pending: Option<PendingMainDestinationEntry>,
}

impl PendingMainDestinationSlot {
    fn install(&mut self, destination: MainWindowDestination) -> PendingMainDestinationAttempt {
        let marker = Arc::new(());
        self.pending = Some(PendingMainDestinationEntry {
            marker: marker.clone(),
            destination,
        });
        PendingMainDestinationAttempt { marker }
    }

    fn rollback(&mut self, attempt: &PendingMainDestinationAttempt) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(&pending.marker, &attempt.marker))
        {
            self.pending.take();
            true
        } else {
            false
        }
    }

    fn take(&mut self) -> Option<MainWindowDestination> {
        self.pending.take().map(|pending| pending.destination)
    }
}

static PENDING_MAIN_DESTINATION: OnceLock<Mutex<PendingMainDestinationSlot>> = OnceLock::new();

fn pending_main_destination() -> &'static Mutex<PendingMainDestinationSlot> {
    PENDING_MAIN_DESTINATION.get_or_init(|| Mutex::new(PendingMainDestinationSlot::default()))
}

fn set_pending_main_window_destination(
    destination: MainWindowDestination,
) -> Result<PendingMainDestinationAttempt, AppError> {
    Ok(pending_main_destination()
        .lock()
        .map_err(|_| AppError::Message("main_navigation_unavailable".to_string()))?
        .install(destination))
}

fn rollback_pending_main_window_destination(
    attempt: &PendingMainDestinationAttempt,
) -> Result<(), AppError> {
    pending_main_destination()
        .lock()
        .map_err(|_| AppError::Message("main_navigation_unavailable".to_string()))?
        .rollback(attempt);
    Ok(())
}

pub fn take_pending_main_window_destination() -> Result<Option<MainWindowDestination>, AppError> {
    Ok(pending_main_destination()
        .lock()
        .map_err(|_| AppError::Message("main_navigation_unavailable".to_string()))?
        .take())
}

#[cfg(test)]
fn clear_pending_main_window_destination_for_test() {
    pending_main_destination()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
}

/// How long the reveal waits for the renderer to report the destination is on
/// screen. Long enough for a settings dialog to mount and paint, short enough
/// that a wedged renderer is a delay rather than a window that never opens.
const REVEAL_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);

type RevealAckSlot = Mutex<Option<tokio::sync::oneshot::Sender<()>>>;

static MAIN_WINDOW_REVEAL_ACK: OnceLock<RevealAckSlot> = OnceLock::new();

fn reveal_ack_slot() -> &'static RevealAckSlot {
    MAIN_WINDOW_REVEAL_ACK.get_or_init(|| Mutex::new(None))
}

/// Arm the wait for the renderer's acknowledgement.
///
/// Dropping a previous sender resolves its receiver with an error, so a reveal
/// that was never acknowledged stops waiting as soon as a newer one starts
/// instead of holding its window back for the full timeout.
fn arm_reveal_ack_in(slot: &RevealAckSlot) -> tokio::sync::oneshot::Receiver<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    if let Ok(mut slot) = slot.lock() {
        *slot = Some(sender);
    }
    receiver
}

fn acknowledge_in(slot: &RevealAckSlot) {
    if let Ok(mut slot) = slot.lock() {
        if let Some(sender) = slot.take() {
            let _ = sender.send(());
        }
    }
}

fn arm_reveal_ack() -> tokio::sync::oneshot::Receiver<()> {
    arm_reveal_ack_in(reveal_ack_slot())
}

/// The renderer reports that the requested destination has been painted.
pub fn acknowledge_main_window_ready() {
    acknowledge_in(reveal_ack_slot());
}

#[cfg(any(target_os = "macos", test))]
pub fn classify_tray_click(button: MouseButton, state: MouseButtonState) -> TrayClickAction {
    match (button, state) {
        (MouseButton::Left, MouseButtonState::Down) => TrayClickAction::TogglePopover,
        (MouseButton::Right, MouseButtonState::Down) => TrayClickAction::HidePopover,
        _ => TrayClickAction::Ignore,
    }
}

#[cfg(any(target_os = "macos", test))]
pub fn calculate_popover_position(
    anchor: PhysicalRect<i32, u32>,
    popup_size: PhysicalSize<u32>,
    work_area: PhysicalRect<i32, u32>,
) -> PhysicalPosition<i32> {
    let centered_x = anchor.position.x + (anchor.size.width as i32 - popup_size.width as i32) / 2;
    let below_y = anchor.position.y + anchor.size.height as i32 + POPOVER_GAP_PHYSICAL;
    let min_x = work_area.position.x;
    let min_y = work_area.position.y;
    let max_x = min_x + work_area.size.width as i32 - popup_size.width as i32;
    let max_y = min_y + work_area.size.height as i32 - popup_size.height as i32;

    PhysicalPosition::new(
        centered_x.clamp(min_x, max_x.max(min_x)),
        below_y.clamp(min_y, max_y.max(min_y)),
    )
}

fn popover_error(operation: &str, error: impl std::fmt::Display) -> AppError {
    log::warn!("tray popover {operation} failed: {error}");
    AppError::Message("tray_popover_unavailable".to_string())
}

fn main_window_error(operation: &str, error: impl std::fmt::Display) -> AppError {
    log::warn!("main window {operation} failed: {error}");
    AppError::Message("main_window_unavailable".to_string())
}

#[cfg(target_os = "macos")]
pub fn ensure_window(app: &AppHandle) -> Result<WebviewWindow, AppError> {
    if let Some(window) = app.get_webview_window(TRAY_POPOVER_LABEL) {
        return Ok(window);
    }

    let window = WebviewWindowBuilder::new(
        app,
        TRAY_POPOVER_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("LLM Usage Bar")
    .inner_size(POPOVER_WIDTH, POPOVER_HEIGHT)
    .visible(false)
    .transparent(true)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .accept_first_mouse(true)
    .build()
    .map_err(|error| popover_error("creation", error))?;

    crate::macos_material::apply_native_material(
        &window,
        crate::macos_material::NativeMaterialSurface::TrayPopover,
    );
    Ok(window)
}

#[cfg(target_os = "macos")]
pub fn toggle(app: &AppHandle, anchor: Rect) -> Result<(), AppError> {
    let window = ensure_window(app)?;
    if window
        .is_visible()
        .map_err(|error| popover_error("visibility check", error))?
    {
        return hide(app);
    }

    let primary_monitor = app
        .primary_monitor()
        .map_err(|error| popover_error("primary monitor query", error))?;
    let provisional_scale = primary_monitor
        .as_ref()
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1.0);
    let provisional_position = anchor.position.to_physical::<f64>(provisional_scale);
    let provisional_size = anchor.size.to_physical::<f64>(provisional_scale);
    let anchor_center = PhysicalPosition::new(
        provisional_position.x + provisional_size.width / 2.0,
        provisional_position.y + provisional_size.height / 2.0,
    );
    let monitor = app
        .monitor_from_point(anchor_center.x, anchor_center.y)
        .map_err(|error| popover_error("anchor monitor query", error))?
        .or(primary_monitor)
        .ok_or_else(|| AppError::Message("tray_monitor_unavailable".to_string()))?;
    let scale = monitor.scale_factor();
    let popup = PhysicalSize::new(
        (POPOVER_WIDTH * scale).round() as u32,
        (POPOVER_HEIGHT * scale).round() as u32,
    );
    let physical_anchor = PhysicalRect {
        position: anchor.position.to_physical::<i32>(scale),
        size: anchor.size.to_physical::<u32>(scale),
    };

    window
        .set_position(calculate_popover_position(
            physical_anchor,
            popup,
            *monitor.work_area(),
        ))
        .map_err(|error| popover_error("positioning", error))?;
    window
        .show()
        .map_err(|error| popover_error("show", error))?;
    window
        .set_focus()
        .map_err(|error| popover_error("focus", error))?;
    let _ = app.emit_to(TRAY_POPOVER_LABEL, "tray-popover-shown", ());
    Ok(())
}

pub fn hide(app: &AppHandle) -> Result<(), AppError> {
    if let Some(window) = app.get_webview_window(TRAY_POPOVER_LABEL) {
        window
            .hide()
            .map_err(|error| popover_error("hide", error))?;
    }
    Ok(())
}

pub fn open_main_window(
    app: &AppHandle,
    destination: MainWindowDestination,
) -> Result<(), AppError> {
    hide(app)?;
    let pending_attempt = set_pending_main_window_destination(destination)?;

    // Settings is a dialog over the usage dashboard, not a page of its own, so
    // revealing first put the dashboard on screen and let the user watch the
    // dialog mount on top of it. A live renderer navigates while the window is
    // still hidden and reports back when the destination is painted; only then
    // is the window worth showing.
    if app.get_webview_window("main").is_some() {
        let acknowledged = arm_reveal_ack();
        let _ = app.emit_to("main", "main-window-navigate", ());

        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            // The result is deliberately ignored: a timeout, a dropped sender
            // and a real acknowledgement all mean the same thing here — stop
            // waiting and show the window.
            let _ = tokio::time::timeout(REVEAL_ACK_TIMEOUT, acknowledged).await;
            if let Err(error) = reveal_main_window(&app) {
                log::error!("Failed to reveal main window after navigation: {error}");
            }
        });
        return Ok(());
    }

    // No window yet: it has to be built before it can navigate, and the
    // renderer drains the destination as it boots.
    if let Err(error) = reveal_main_window(app) {
        if let Err(rollback_error) = rollback_pending_main_window_destination(&pending_attempt) {
            log::warn!("failed to rollback pending main navigation: {rollback_error}");
        }
        return Err(error);
    }
    let _ = app.emit_to("main", "main-window-navigate", ());
    Ok(())
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MainWindowRevealRollback {
    hide_main_window: bool,
    restore_dock_visibility: Option<bool>,
}

#[cfg(target_os = "macos")]
fn main_window_reveal_rollback(previous_dock_visible: bool) -> MainWindowRevealRollback {
    if previous_dock_visible {
        // The caller did not expose the app in the Dock. A later reveal error
        // must not hide an already-visible main window or demote the app to an
        // Accessory activation policy.
        MainWindowRevealRollback {
            hide_main_window: false,
            restore_dock_visibility: None,
        }
    } else {
        MainWindowRevealRollback {
            hide_main_window: true,
            restore_dock_visibility: Some(false),
        }
    }
}

pub fn reveal_main_window(app: &AppHandle) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    let previous_dock_visible = crate::tray::try_apply_tray_policy(app, true)
        .map_err(|error| main_window_error("activation policy", error))?;

    let result = (|| {
        crate::lightweight::exit_lightweight_mode(app)
            .map_err(|error| main_window_error("lightweight-mode exit", error))?;
        let main = app
            .get_webview_window("main")
            .ok_or_else(|| AppError::Message("main_window_unavailable".to_string()))?;

        #[cfg(target_os = "windows")]
        main.set_skip_taskbar(false)
            .map_err(|error| main_window_error("taskbar restore", error))?;

        main.unminimize()
            .map_err(|error| main_window_error("unminimize", error))?;
        main.show()
            .map_err(|error| main_window_error("show", error))?;
        main.set_focus()
            .map_err(|error| main_window_error("focus", error))?;

        #[cfg(target_os = "linux")]
        crate::linux_fix::nudge_main_window(main);
        #[cfg(target_os = "macos")]
        crate::macos_fix::repair_undersized_window(main);

        Ok(())
    })();

    #[cfg(target_os = "macos")]
    if result.is_err() {
        let rollback = main_window_reveal_rollback(previous_dock_visible);
        if rollback.hide_main_window {
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.hide();
            }
        }
        if let Some(previous) = rollback.restore_dock_visibility {
            if let Err(error) = crate::tray::try_apply_tray_policy(app, previous) {
                log::warn!("failed to restore tray activation policy: {error}");
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::tray::{MouseButton, MouseButtonState};
    use tauri::{PhysicalPosition, PhysicalRect, PhysicalSize};

    fn physical_rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect<i32, u32> {
        PhysicalRect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    // The ack slot is exercised on its own rather than through the process-wide
    // one: these tests run in parallel, and a shared slot would have them arm
    // and drop each other's senders.
    fn ack_slot() -> RevealAckSlot {
        Mutex::new(None)
    }

    #[tokio::test]
    async fn the_renderer_releases_the_reveal_it_was_armed_for() {
        let slot = ack_slot();
        let acknowledged = arm_reveal_ack_in(&slot);
        acknowledge_in(&slot);
        assert_eq!(acknowledged.await, Ok(()));
    }

    #[tokio::test]
    async fn a_newer_reveal_stops_the_previous_one_from_waiting_out_its_timeout() {
        let slot = ack_slot();
        let stale = arm_reveal_ack_in(&slot);
        let current = arm_reveal_ack_in(&slot);

        // Arming again drops the previous sender, which resolves its receiver
        // with an error — the stale reveal proceeds instead of holding its
        // window back for the full timeout.
        assert!(stale.await.is_err());

        acknowledge_in(&slot);
        assert_eq!(current.await, Ok(()));
    }

    #[tokio::test]
    async fn an_acknowledgement_nobody_asked_for_is_ignored() {
        let slot = ack_slot();
        acknowledge_in(&slot);

        // In particular it must not leave a permit behind that lets the next
        // reveal skip its wait and show the window mid-navigation.
        let acknowledged = arm_reveal_ack_in(&slot);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), acknowledged)
                .await
                .is_err()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reveal_rollback_preserves_an_already_visible_main_window_and_dock() {
        assert_eq!(
            main_window_reveal_rollback(true),
            MainWindowRevealRollback {
                hide_main_window: false,
                restore_dock_visibility: None,
            }
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reveal_rollback_hides_and_restores_a_tray_only_app() {
        assert_eq!(
            main_window_reveal_rollback(false),
            MainWindowRevealRollback {
                hide_main_window: true,
                restore_dock_visibility: Some(false),
            }
        );
    }

    #[test]
    fn only_left_down_toggles_and_right_down_hides() {
        assert_eq!(
            classify_tray_click(MouseButton::Left, MouseButtonState::Down),
            TrayClickAction::TogglePopover,
        );
        assert_eq!(
            classify_tray_click(MouseButton::Left, MouseButtonState::Up),
            TrayClickAction::Ignore,
        );
        assert_eq!(
            classify_tray_click(MouseButton::Right, MouseButtonState::Down),
            TrayClickAction::HidePopover,
        );
        assert_eq!(
            classify_tray_click(MouseButton::Right, MouseButtonState::Up),
            TrayClickAction::Ignore,
        );
        assert_eq!(
            classify_tray_click(MouseButton::Middle, MouseButtonState::Down),
            TrayClickAction::Ignore,
        );
    }

    #[test]
    fn popover_is_centered_below_anchor_and_clamped_to_work_area_at_2x() {
        let position = calculate_popover_position(
            physical_rect(1900, 0, 22, 24),
            PhysicalSize::new(760, 1040),
            physical_rect(0, 0, 1920, 1080),
        );

        assert_eq!(position.x, 1160);
        assert_eq!(position.y, 32);
    }

    #[test]
    fn popover_is_centered_below_anchor_at_1x() {
        let position = calculate_popover_position(
            physical_rect(940, 0, 40, 24),
            PhysicalSize::new(380, 520),
            physical_rect(0, 0, 1920, 1080),
        );

        assert_eq!(position, PhysicalPosition::new(770, 32));
    }

    #[test]
    fn popover_clamps_each_work_area_edge() {
        let popup = PhysicalSize::new(380, 520);
        let work_area = physical_rect(0, 100, 1920, 900);

        assert_eq!(
            calculate_popover_position(physical_rect(-200, 200, 20, 24), popup, work_area).x,
            0,
        );
        assert_eq!(
            calculate_popover_position(physical_rect(1900, 200, 20, 24), popup, work_area).x,
            1540,
        );
        assert_eq!(
            calculate_popover_position(physical_rect(950, 50, 20, 24), popup, work_area).y,
            100,
        );
        assert_eq!(
            calculate_popover_position(physical_rect(950, 900, 20, 24), popup, work_area).y,
            480,
        );
    }

    #[test]
    fn popover_clamps_with_a_negative_secondary_monitor_origin() {
        let position = calculate_popover_position(
            physical_rect(-10, 0, 20, 24),
            PhysicalSize::new(380, 520),
            physical_rect(-1920, 0, 1920, 1080),
        );

        assert_eq!(position, PhysicalPosition::new(-380, 32));
    }

    #[test]
    fn anchor_wider_than_popup_still_centers_the_popover() {
        let position = calculate_popover_position(
            physical_rect(100, 0, 500, 24),
            PhysicalSize::new(380, 520),
            physical_rect(0, 0, 1920, 1080),
        );

        assert_eq!(position, PhysicalPosition::new(160, 32));
    }

    #[test]
    fn work_area_smaller_than_popup_pins_to_its_origin() {
        let position = calculate_popover_position(
            physical_rect(50, 50, 20, 20),
            PhysicalSize::new(380, 520),
            physical_rect(-100, -200, 200, 300),
        );

        assert_eq!(position, PhysicalPosition::new(-100, -200));
    }

    #[test]
    fn main_window_destinations_serialize_as_typed_camel_case_payloads() {
        let usage = MainWindowDestination::Usage {
            agent_module_id: Some("codex".to_string()),
        };
        let budget = MainWindowDestination::ProviderBudget {
            provider_id: Some("openai-api".to_string()),
        };

        assert_eq!(
            serde_json::to_value(usage).unwrap(),
            serde_json::json!({"kind": "usage", "agentModuleId": "codex"}),
        );
        assert_eq!(
            serde_json::to_value(budget).unwrap(),
            serde_json::json!({"kind": "providerBudget", "providerId": "openai-api"}),
        );
    }

    #[test]
    fn pending_main_window_destination_is_consumed_once() {
        clear_pending_main_window_destination_for_test();
        let destination = MainWindowDestination::Usage {
            agent_module_id: None,
        };

        set_pending_main_window_destination(destination.clone()).unwrap();

        assert_eq!(
            take_pending_main_window_destination().unwrap(),
            Some(destination)
        );
        assert_eq!(take_pending_main_window_destination().unwrap(), None);
    }

    #[test]
    fn failed_reveal_rolls_back_the_destination_installed_by_that_attempt() {
        let mut slot = PendingMainDestinationSlot::default();
        let attempt = slot.install(MainWindowDestination::Usage {
            agent_module_id: Some("codex".to_string()),
        });

        slot.rollback(&attempt);

        assert_eq!(slot.take(), None);
    }

    #[test]
    fn older_failed_reveal_does_not_clobber_a_newer_destination() {
        let mut slot = PendingMainDestinationSlot::default();
        let older_attempt = slot.install(MainWindowDestination::Usage {
            agent_module_id: Some("codex".to_string()),
        });
        let newer_destination = MainWindowDestination::ProviderBudget {
            provider_id: Some("openai-api".to_string()),
        };
        let _newer_attempt = slot.install(newer_destination.clone());

        slot.rollback(&older_attempt);

        assert_eq!(slot.take(), Some(newer_destination));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn successful_pending_transaction_is_consumed_once() {
        let mut slot = PendingMainDestinationSlot::default();
        let destination = MainWindowDestination::Usage {
            agent_module_id: None,
        };
        let _attempt = slot.install(destination.clone());

        assert_eq!(slot.take(), Some(destination));
        assert_eq!(slot.take(), None);
    }
}
