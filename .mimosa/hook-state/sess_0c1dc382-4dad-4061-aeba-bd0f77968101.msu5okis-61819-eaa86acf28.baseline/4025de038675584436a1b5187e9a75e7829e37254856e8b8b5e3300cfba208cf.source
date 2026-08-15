#[cfg(target_os = "macos")]
use objc2_web_kit::WKWebView;
#[cfg(target_os = "macos")]
use tauri::WebviewWindow;
#[cfg(target_os = "macos")]
use window_vibrancy::{
    apply_liquid_glass, apply_vibrancy, clear_liquid_glass, clear_vibrancy, LiquidGlassOptions,
    NSGlassEffectViewStyle, NSVisualEffectMaterial, NSVisualEffectState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeMaterialSurface {
    MainWindow,
    TrayPopover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FallbackMaterial {
    WindowBackground,
    Popover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FallbackState {
    FollowsWindowActiveState,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct NativeMaterialSpec {
    log_label: &'static str,
    radius: Option<f64>,
    fallback_material: FallbackMaterial,
    fallback_state: FallbackState,
}

impl NativeMaterialSurface {
    fn spec(self) -> NativeMaterialSpec {
        match self {
            Self::MainWindow => NativeMaterialSpec {
                log_label: "main window",
                radius: None,
                fallback_material: FallbackMaterial::WindowBackground,
                fallback_state: FallbackState::FollowsWindowActiveState,
            },
            Self::TrayPopover => NativeMaterialSpec {
                log_label: "tray popover",
                radius: Some(16.0),
                fallback_material: FallbackMaterial::Popover,
                fallback_state: FallbackState::Active,
            },
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_native_material(window: &WebviewWindow, surface: NativeMaterialSurface) {
    let spec = surface.spec();
    let material_window = window.clone();

    if let Err(error) = window.with_webview(move |webview| {
        if let Err(error) = clear_liquid_glass(&material_window) {
            log::warn!(
                "{} existing Liquid Glass cleanup failed: {error}",
                spec.log_label
            );
        }
        if let Err(error) = clear_vibrancy(&material_window) {
            log::warn!(
                "{} existing vibrancy cleanup failed: {error}",
                spec.log_label
            );
        }

        let apply_fallback = || {
            let material = match spec.fallback_material {
                FallbackMaterial::WindowBackground => NSVisualEffectMaterial::WindowBackground,
                FallbackMaterial::Popover => NSVisualEffectMaterial::Popover,
            };
            let state = match spec.fallback_state {
                FallbackState::FollowsWindowActiveState => {
                    NSVisualEffectState::FollowsWindowActiveState
                }
                FallbackState::Active => NSVisualEffectState::Active,
            };

            match apply_vibrancy(&material_window, material, Some(state), spec.radius) {
                Ok(()) => log::info!("{} native material: vibrancy fallback", spec.log_label),
                Err(error) => log::warn!("{} vibrancy fallback failed: {error}", spec.log_label),
            }
        };

        // Tauri exposes the platform webview as WKWebView on macOS. It is
        // borrowed only for this main-thread callback, then safely upcast to
        // NSView by LiquidGlassOptions::content_view.
        let content_view = unsafe { webview.inner().cast::<WKWebView>().as_ref() };
        let Some(content_view) = content_view else {
            log::warn!(
                "{} Liquid Glass skipped: WKWebView pointer was null",
                spec.log_label
            );
            apply_fallback();
            return;
        };

        let mut options = LiquidGlassOptions::new(NSGlassEffectViewStyle::Regular)
            .opaque(false)
            .content_view(content_view);
        if let Some(radius) = spec.radius {
            options = options.radius(radius);
        }

        match apply_liquid_glass(&material_window, options) {
            Ok(()) => log::info!("{} native material: Liquid Glass", spec.log_label),
            Err(window_vibrancy::Error::UnsupportedPlatformVersion(_)) => {
                log::debug!(
                    "{} Liquid Glass unavailable; using vibrancy fallback",
                    spec.log_label
                );
                apply_fallback();
            }
            Err(error) => {
                log::warn!(
                    "{} Liquid Glass failed: {error}; using vibrancy fallback",
                    spec.log_label
                );
                apply_fallback();
            }
        }
    }) {
        log::warn!(
            "{} native material setup failed: {error}",
            surface.spec().log_label
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_window_uses_window_material_without_custom_radius() {
        let spec = NativeMaterialSurface::MainWindow.spec();

        assert_eq!(spec.log_label, "main window");
        assert_eq!(spec.radius, None);
        assert_eq!(spec.fallback_material, FallbackMaterial::WindowBackground);
        assert_eq!(spec.fallback_state, FallbackState::FollowsWindowActiveState);
    }

    #[test]
    fn tray_popover_keeps_active_popover_material_and_corner_radius() {
        let spec = NativeMaterialSurface::TrayPopover.spec();

        assert_eq!(spec.log_label, "tray popover");
        assert_eq!(spec.radius, Some(16.0));
        assert_eq!(spec.fallback_material, FallbackMaterial::Popover);
        assert_eq!(spec.fallback_state, FallbackState::Active);
    }
}
