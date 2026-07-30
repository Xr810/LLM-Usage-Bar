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

    #[test]
    fn bundled_status_icons_are_single_lamp_signals() {
        for (status, expected_rgb) in [
            (UsageStatus::Green, [0x34, 0xC7, 0x59]),
            (UsageStatus::Yellow, [0xFF, 0xCC, 0x00]),
            (UsageStatus::Red, [0xFF, 0x3B, 0x30]),
            (UsageStatus::Unknown, [0x8E, 0x8E, 0x93]),
        ] {
            let image = decode_status_icon(status).unwrap();
            assert_eq!((image.width(), image.height()), (18, 18), "{status:?}");
            assert_eq!(image.rgba().len(), 18 * 18 * 4, "{status:?}");

            for (x, y) in [(0, 0), (17, 0), (0, 17), (17, 17)] {
                assert_eq!(pixel(&image, x, y)[3], 0, "{status:?} corner {x},{y}");
            }
            for (x, y) in [(8, 8), (9, 8), (8, 9), (9, 9)] {
                let rgba = pixel(&image, x, y);
                assert_eq!(&rgba[..3], &expected_rgb, "{status:?} center {x},{y}");
                assert_eq!(rgba[3], 255, "{status:?} center {x},{y}");
            }

            assert!(
                image
                    .rgba()
                    .chunks_exact(4)
                    .any(|rgba| (1..=254).contains(&rgba[3])),
                "{status:?} must retain an antialiased edge"
            );

            let housing = pixel(&image, 9, 2);
            assert!(
                housing[3] >= 200,
                "{status:?} housing must be visible above the lamp"
            );
            assert!(
                housing[..3].iter().all(|channel| *channel <= 80),
                "{status:?} housing must remain dark"
            );

            let nonzero = (0..18)
                .flat_map(|y| (0..18).map(move |x| (x, y)))
                .filter(|&(x, y)| pixel(&image, x, y)[3] != 0)
                .collect::<Vec<_>>();
            let min_x = nonzero.iter().map(|(x, _)| *x).min().unwrap();
            let max_x = nonzero.iter().map(|(x, _)| *x).max().unwrap();
            let min_y = nonzero.iter().map(|(_, y)| *y).min().unwrap();
            let max_y = nonzero.iter().map(|(_, y)| *y).max().unwrap();
            let width = max_x - min_x + 1;
            let height = max_y - min_y + 1;
            assert!((14..=16).contains(&width), "{status:?} width {width}");
            assert!((16..=18).contains(&height), "{status:?} height {height}");
            assert_eq!(min_x + max_x, 17, "{status:?} horizontal center");
            assert_eq!(min_y + max_y, 17, "{status:?} vertical center");
        }
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
