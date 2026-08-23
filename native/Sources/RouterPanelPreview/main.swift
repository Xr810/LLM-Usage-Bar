// 一个只为「把四个分区跑起来看一眼」而存在的壳。
//
// 它不是产品外壳:没有菜单栏、没有生命周期、不连真实 bridge,数据来自
// PreviewRouterRepository。产品外壳是另一件事。
//
// 用法:swift run RouterPanelPreview [--theme system|overcast|ink] [--opaque] [--empty]
import AppKit
import NativeUI
import SwiftUI
import UsageCore

private struct Options {
    var themeId = "system"
    var appearance: NativeAppearance = .light
    var translucent = true
    var empty = false

    static func parse(_ arguments: [String]) -> Options {
        var options = Options()
        var iterator = arguments.dropFirst().makeIterator()
        while let argument = iterator.next() {
            switch argument {
            case "--theme":
                if let value = iterator.next() {
                    options.themeId = value
                    options.appearance = (value == "ink") ? .dark : .light
                }
            case "--opaque":
                options.translucent = false
            case "--empty":
                options.empty = true
            default:
                break
            }
        }
        return options
    }
}

private let options = Options.parse(CommandLine.arguments)

private let repository = options.empty
    ? PreviewRouterRepository(providers: [], routes: [], attempts: [])
    : PreviewRouterRepository()

@MainActor
private func makeModel() -> RouterPanelModel {
    let model = RouterPanelModel(repository: repository)
    Task { await model.load(range: DashboardRangeV1(startAt: 0, endAt: 1)) }
    return model
}

private let settings = NativeAppearanceSettings(
    lightThemeId: options.appearance == .dark ? "system" : options.themeId,
    darkThemeId: options.appearance == .dark ? options.themeId : "system",
    translucencyEnabled: options.translucent,
    followsReduceTransparency: true
)

private let resolved = settings.resolve(
    appearance: options.appearance,
    systemReduceTransparency: false
)

private final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let view = RouterPanelView(model: makeModel(), appearance: resolved)
        let hosting = NSHostingView(rootView: view)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 860, height: 760),
            styleMask: [.titled, .closable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "路由 · 预览(\(options.themeId))"
        window.contentView = hosting
        window.appearance = NSAppearance(
            named: options.appearance == .dark ? .darkAqua : .aqua
        )
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window
        NSApp.activate(ignoringOtherApps: true)
    }
}

private let app = NSApplication.shared
private let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
