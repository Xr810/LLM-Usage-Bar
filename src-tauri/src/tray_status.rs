use crate::usage::status::UsageStatus;
use crate::usage::tray_snapshot::TrayUsageSnapshot;
#[cfg(any(target_os = "macos", test))]
use tauri::image::Image;
use tauri::{AppHandle, Emitter};

pub const EVENT_TRAY_USAGE_UPDATED: &str = "tray-usage-updated";

pub fn tray_status_tooltip(status: UsageStatus) -> &'static str {
    match status {
        UsageStatus::Green => "LLM Usage Bar — Usage healthy",
        UsageStatus::Yellow => "LLM Usage Bar — Usage warning",
        UsageStatus::Red => "LLM Usage Bar — Usage critical",
        UsageStatus::Unknown => "LLM Usage Bar — Data unavailable",
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn status_icon_bytes(status: UsageStatus) -> &'static [u8] {
    match status {
        UsageStatus::Green => include_bytes!("../icons/tray/macos/status_green.png"),
        UsageStatus::Yellow => include_bytes!("../icons/tray/macos/status_yellow.png"),
        UsageStatus::Red => include_bytes!("../icons/tray/macos/status_red.png"),
        UsageStatus::Unknown => include_bytes!("../icons/tray/macos/status_unknown.png"),
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn decode_status_icon(status: UsageStatus) -> Result<Image<'static>, tauri::Error> {
    Image::from_bytes(status_icon_bytes(status))
}

fn publish_tray_usage_with_sinks<Icon, Tooltip, Event>(
    snapshot: &TrayUsageSnapshot,
    mut update_icon: Icon,
    mut update_tooltip: Tooltip,
    mut emit_event: Event,
) where
    Icon: FnMut(UsageStatus) -> Result<(), ()>,
    Tooltip: FnMut(&'static str) -> Result<(), ()>,
    Event: FnMut(TrayUsageSnapshot) -> Result<(), ()>,
{
    let status = snapshot.status;
    let _ = update_icon(status);
    let _ = update_tooltip(tray_status_tooltip(status));
    let _ = emit_event(snapshot.clone());
}

#[cfg(target_os = "macos")]
fn try_update_tray_status_icon(app: &AppHandle, status: UsageStatus) -> Result<(), ()> {
    let icon = decode_status_icon(status).map_err(|_| {
        log::warn!("tray status icon decode failed");
    })?;
    let tray = app.tray_by_id(crate::tray::TRAY_ID).ok_or_else(|| {
        log::warn!("tray status icon is unavailable");
    })?;
    tray.set_icon_with_as_template(Some(icon), false)
        .map_err(|_| {
            log::warn!("tray status icon update failed");
        })
}

pub fn update_tray_status_icon(app: &AppHandle, status: UsageStatus) {
    #[cfg(target_os = "macos")]
    {
        let _ = try_update_tray_status_icon(app, status);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, status);
    }
}

fn try_update_tray_tooltip(app: &AppHandle, tooltip: &'static str) -> Result<(), ()> {
    let tray = app.tray_by_id(crate::tray::TRAY_ID).ok_or_else(|| {
        log::warn!("tray status tooltip is unavailable");
    })?;
    tray.set_tooltip(Some(tooltip)).map_err(|_| {
        log::warn!("tray status tooltip update failed");
    })
}

pub fn publish_tray_usage(app: &AppHandle, snapshot: &TrayUsageSnapshot) {
    #[cfg(target_os = "macos")]
    if let Err(error) = crate::native_bridge::persist_tray_snapshot(snapshot) {
        log::warn!("native tray snapshot persistence failed: {error}");
    }

    crate::services::budget_alert::notify_for_snapshot(app, snapshot);
    publish_tray_usage_with_sinks(
        snapshot,
        |status| {
            update_tray_status_icon(app, status);
            Ok(())
        },
        |tooltip| try_update_tray_tooltip(app, tooltip),
        |payload| {
            app.emit(EVENT_TRAY_USAGE_UPDATED, payload).map_err(|_| {
                log::warn!("tray usage event emit failed");
            })
        },
    );
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn select_initial_status_icon<T, E, Legacy>(
    unknown: Result<T, E>,
    legacy: Legacy,
) -> Option<(T, bool)>
where
    Legacy: FnOnce() -> Option<T>,
{
    match unknown {
        Ok(icon) => Some((icon, false)),
        Err(_) => legacy().map(|icon| (icon, true)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn pixel(image: &Image<'_>, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * image.width() as usize + x) * 4;
        image.rgba()[offset..offset + 4].try_into().unwrap()
    }

    #[test]
    fn tooltips_are_exact_and_accessible() {
        assert_eq!(
            tray_status_tooltip(UsageStatus::Green),
            "LLM Usage Bar — Usage healthy"
        );
        assert_eq!(
            tray_status_tooltip(UsageStatus::Yellow),
            "LLM Usage Bar — Usage warning"
        );
        assert_eq!(
            tray_status_tooltip(UsageStatus::Red),
            "LLM Usage Bar — Usage critical"
        );
        assert_eq!(
            tray_status_tooltip(UsageStatus::Unknown),
            "LLM Usage Bar — Data unavailable"
        );
    }

    /// Lamp centres measured from the generated assets. `scripts/generate_icons.py`
    /// authors the mark on a 36px-tall grid, which is an 18pt menu bar slot at 2x.
    const LAMP_CENTRES_X: [u32; 3] = [17, 45, 73];
    const LAMP_CENTRE_Y: u32 = 17;

    #[test]
    fn bundled_status_icons_light_the_lamp_that_names_the_status() {
        for (status, lamp, expected_rgb) in [
            (UsageStatus::Red, Some(0), [0xFF, 0x45, 0x3A]),
            (UsageStatus::Yellow, Some(1), [0xFF, 0xB0, 0x20]),
            (UsageStatus::Green, Some(2), [0x30, 0xD1, 0x58]),
            (UsageStatus::Unknown, None, [0, 0, 0]),
        ] {
            let image = decode_status_icon(status).unwrap();
            assert_eq!((image.width(), image.height()), (92, 36), "{status:?}");
            assert_eq!(image.rgba().len(), 92 * 36 * 4, "{status:?}");

            // The housing is a rounded rectangle, so the corners are clear.
            for (x, y) in [(0, 0), (91, 0), (0, 35), (91, 35)] {
                assert_eq!(pixel(&image, x, y)[3], 0, "{status:?} corner {x},{y}");
            }

            // Status is carried by *which* lamp is lit as well as by its hue, so
            // it survives a colour-blind reader and a greyscale screenshot. That
            // only holds if each status lights its own position and no other.
            for (index, centre_x) in LAMP_CENTRES_X.iter().enumerate() {
                let rgba = pixel(&image, *centre_x as usize, LAMP_CENTRE_Y as usize);
                if lamp == Some(index) {
                    assert_eq!(&rgba[..3], &expected_rgb, "{status:?} lamp {index}");
                    assert_eq!(rgba[3], 255, "{status:?} lamp {index} must be opaque");
                } else {
                    // An unlit lamp is an outline, so its centre is empty.
                    assert_eq!(rgba[3], 0, "{status:?} lamp {index} must stay unlit");
                }
            }

            assert!(
                image
                    .rgba()
                    .chunks_exact(4)
                    .any(|rgba| (1..=254).contains(&rgba[3])),
                "{status:?} must retain an antialiased edge"
            );
        }
    }

    #[test]
    fn bundled_status_icons_keep_structure_readable_on_either_menu_bar() {
        // The icon carries colour, so it cannot be a template image and macOS
        // will not invert it for the bar's appearance. Structure therefore has
        // to be a grey far enough from both bar colours to survive either one.
        let image = decode_status_icon(UsageStatus::Unknown).unwrap();
        let housing = pixel(&image, 46, 1);
        assert!(
            housing[3] >= 200,
            "housing must be opaque at the top edge, got alpha {}",
            housing[3]
        );
        let luminance = i32::from(housing[0]);
        assert!(
            (90..=160).contains(&luminance),
            "housing grey {luminance} must clear both a light and a dark bar"
        );
    }

    #[test]
    fn publisher_uses_one_snapshot_in_icon_tooltip_event_order() {
        let mut snapshot = TrayUsageSnapshot::unknown(42);
        snapshot.status = UsageStatus::Yellow;
        let calls = Rc::new(RefCell::new(Vec::new()));

        publish_tray_usage_with_sinks(
            &snapshot,
            {
                let calls = calls.clone();
                move |status| {
                    calls.borrow_mut().push(format!("icon:{status:?}"));
                    Ok(())
                }
            },
            {
                let calls = calls.clone();
                move |tooltip| {
                    calls.borrow_mut().push(format!("tooltip:{tooltip}"));
                    Ok(())
                }
            },
            {
                let calls = calls.clone();
                move |payload| {
                    calls.borrow_mut().push(format!(
                        "event:{}",
                        serde_json::to_string(&payload).unwrap()
                    ));
                    Ok(())
                }
            },
        );

        assert_eq!(
            calls.borrow().as_slice(),
            [
                "icon:Yellow".to_string(),
                "tooltip:LLM Usage Bar — Usage warning".to_string(),
                format!("event:{}", serde_json::to_string(&snapshot).unwrap()),
            ]
        );
    }

    #[test]
    fn publisher_failure_does_not_suppress_later_sinks() {
        let snapshot = TrayUsageSnapshot::unknown(42);
        let calls = Rc::new(RefCell::new(Vec::new()));

        publish_tray_usage_with_sinks(
            &snapshot,
            {
                let calls = calls.clone();
                move |_| {
                    calls.borrow_mut().push("icon".to_string());
                    Err(())
                }
            },
            {
                let calls = calls.clone();
                move |_| {
                    calls.borrow_mut().push("tooltip".to_string());
                    Err(())
                }
            },
            {
                let calls = calls.clone();
                move |payload| {
                    calls
                        .borrow_mut()
                        .push(format!("event:{:?}", payload.status));
                    Ok(())
                }
            },
        );

        assert_eq!(
            calls.borrow().as_slice(),
            ["icon", "tooltip", "event:Unknown"]
        );
    }

    #[test]
    fn initial_unknown_icon_only_uses_legacy_as_fallback() {
        let legacy_called = Cell::new(false);
        assert_eq!(
            select_initial_status_icon::<_, &str, _>(Ok(7), || {
                legacy_called.set(true);
                Some(8)
            }),
            Some((7, false))
        );
        assert!(!legacy_called.get());

        assert_eq!(
            select_initial_status_icon::<_, &str, _>(Err("decode"), || Some(8)),
            Some((8, true))
        );
        assert_eq!(
            select_initial_status_icon::<i32, _, _>(Err("decode"), || None),
            None
        );
    }

    #[test]
    fn event_name_is_stable() {
        assert_eq!(EVENT_TRAY_USAGE_UPDATED, "tray-usage-updated");
    }
}
