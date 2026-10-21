#if os(macOS)
import AppKit
import SwiftUI

@MainActor
final class AppLifecycle {
    static let shared = AppLifecycle()

    func enterBackgroundMode() {
        #if NATIVE_PREVIEW
        return
        #else
        guard !hasVisiblePrimaryWindow else { return }
        NSApplication.shared.setActivationPolicy(.accessory)
        #endif
    }

    func enterForegroundMode() {
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

    func openSettingsWindow() {
        enterForegroundMode()
        if !NSApplication.shared.sendAction(
            Selector(("showSettingsWindow:")),
            to: nil,
            from: nil
        ) {
            _ = NSApplication.shared.sendAction(
                Selector(("showPreferencesWindow:")),
                to: nil,
                from: nil
            )
        }
    }

    private var hasVisiblePrimaryWindow: Bool {
        NSApplication.shared.windows.contains { window in
            window.isVisible && !(window is NSPanel)
        }
    }
}

final class NativeAppDelegate: NSObject, NSApplicationDelegate {
    private var windowCloseObserver: NSObjectProtocol?

    func applicationDidFinishLaunching(_ notification: Notification) {
        #if NATIVE_PREVIEW
        NSApplication.shared.setActivationPolicy(.regular)
        #else
        NSApplication.shared.setActivationPolicy(.accessory)
        #endif
        windowCloseObserver = NotificationCenter.default.addObserver(
            forName: Notification.Name("NSWindowDidCloseNotification"),
            object: nil,
            queue: .main
        ) { _ in
            Task { @MainActor in
                await Task.yield()
                AppLifecycle.shared.enterBackgroundMode()
            }
        }
        #if NATIVE_PREVIEW
        DispatchQueue.main.async {
            AppLifecycle.shared.enterForegroundMode()
            NSApplication.shared.windows
                .first { !($0 is NSPanel) }
                .map { window in
                    window.makeKeyAndOrderFront(nil)
                }
        }
        #else
        // A `Window(id:)` is retained for later `openWindow`, but the menu-bar
        // preview must not reveal it during launch.
        DispatchQueue.main.async {
            NSApplication.shared.windows
                .filter { !($0 is NSPanel) }
                .forEach { $0.orderOut(nil) }
        }
        #endif
    }

    deinit {
        if let windowCloseObserver {
            NotificationCenter.default.removeObserver(windowCloseObserver)
        }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if let window = sender.windows.first(where: { !($0 is NSPanel) }) {
            AppLifecycle.shared.enterForegroundMode()
            window.makeKeyAndOrderFront(nil)
        }
        return true
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}

struct NativeAppCommands: Commands {
    @Environment(\.openWindow) private var openWindow
    @ObservedObject var model: UsageAppModel

    var body: some Commands {
        CommandGroup(after: .appInfo) {
            Button(L10n.text(.details)) {
                AppLifecycle.shared.enterForegroundMode()
                openWindow(id: "main")
            }
            .keyboardShortcut("1", modifiers: .command)
        }
        CommandGroup(replacing: .appTermination) {
            Button(L10n.text(.quit)) { Task { await model.quit() } }
                .keyboardShortcut("q", modifiers: .command)
        }
    }
}
#endif
