#if os(macOS)
import AppKit
import SwiftUI

@main
struct LLMUsageBarNativeApp: App {
    @NSApplicationDelegateAdaptor(NativeAppDelegate.self) private var appDelegate
    @StateObject private var model = UsageAppModel()

    var body: some Scene {
        MenuBarExtra {
            UsageMenuView(model: model)
        } label: {
            StatusLight(status: model.snapshot.status, size: 12)
                .accessibilityLabel(model.statusAccessibilityLabel)
                #if NATIVE_PREVIEW
                .background { NativePreviewWindowOpener() }
                #endif
        }
        .menuBarExtraStyle(.window)

        Window(L10n.text(.details), id: "main") {
            MainWindowView(model: model)
                .frame(minWidth: 900, minHeight: 600)
                .onAppear { AppLifecycle.shared.enterForegroundMode() }
        }
        .defaultSize(width: 1000, height: 650)
        .windowResizability(.contentMinSize)
        .commands { NativeAppCommands(model: model) }

        Settings {
            NativeSettingsView(model: model)
                .frame(width: 820, height: 560)
        }
    }
}

#if NATIVE_PREVIEW
private struct NativePreviewWindowOpener: View {
    @Environment(\.openWindow) private var openWindow
    @State private var hasOpened = false

    var body: some View {
        Color.clear
            .frame(width: 0, height: 0)
            .task {
                guard !hasOpened,
                      ProcessInfo.processInfo.environment["XCODE_RUNNING_FOR_PREVIEWS"] != "1"
                else { return }
                hasOpened = true
                await Task.yield()
                AppLifecycle.shared.enterForegroundMode()
                openWindow(id: "main")
            }
    }
}
#endif
#else
import Foundation

@main
enum LLMUsageBarNativeApp {
    static func main() {
        print("LLMUsageBarNative requires macOS 13 or later.")
    }
}
#endif
