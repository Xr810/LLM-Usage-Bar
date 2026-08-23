// 无头渲染:把视图直接渲成 PNG,不需要 Xcode、不需要屏幕权限、不需要窗口。
//
// **为什么长在测试 target 里**:它原本是个可执行 target,但那样 Xcode 会挑它当
// SwiftUI 预览的宿主,而新版 Xcode 要求可执行 target 开 `ENABLE_DEBUG_DYLIB` ——
// SwiftPM 设不了这个开关,结果四个 #Preview 全部渲染失败(DebugDylibNotEnabled)。
// 测试 target 不会被选作预览宿主,搬到这里之后预览与无头渲染两条路都通。
//
// 用法(不设环境变量时这条测试直接跳过,不会在每次 swift test 时往磁盘写东西):
//   PANEL_SHOTS_DIR=/tmp/shots swift test --filter renderPanelSnapshots
//
// 已知局限(渲图时要绕开,不要拿渲染结果去判断这两件事):
//   1. `.regularMaterial` 背后没有窗口,材质会渲成近乎透明 —— 所以只渲不透明主题
//   2. `List` 在 ImageRenderer 下不保证出内容,自动模式的排序列表可能是空的
import AppKit
import SwiftUI
import Testing
import UsageCore
@testable import NativeUI

@MainActor
enum PanelRenderer {
    struct Shot {
        let name: String
        let themeId: String
        let appearance: NativeAppearance
        let repository: PreviewRouterRepository
        let mutate: (RouterPanelModel) async -> Void

        init(
            name: String,
            themeId: String,
            appearance: NativeAppearance,
            repository: PreviewRouterRepository = PreviewRouterRepository(),
            mutate: @escaping (RouterPanelModel) async -> Void = { _ in }
        ) {
            self.name = name
            self.themeId = themeId
            self.appearance = appearance
            self.repository = repository
            self.mutate = mutate
        }
    }

    static func shots() -> [Shot] {
        [
            Shot(name: "01-default-light", themeId: "system", appearance: .light),
            Shot(name: "02-overcast-light", themeId: "overcast", appearance: .light),
            Shot(name: "03-ink-dark", themeId: "ink", appearance: .dark),
            Shot(
                name: "04-empty",
                themeId: "system",
                appearance: .light,
                repository: PreviewRouterRepository(providers: [], routes: [], attempts: [])
            ),
            Shot(name: "05-manual-mode", themeId: "overcast", appearance: .light) { model in
                await model.setMode(.manual(providerId: "sol-relay"))
            },
            Shot(name: "06-after-takeover", themeId: "overcast", appearance: .light) { model in
                await model.enablePointer()
            },
            Shot(name: "07-error-banner", themeId: "overcast", appearance: .light) { model in
                model.errorMessage = "非法 wire_api: rest。这一行的协议只能是 responses 或 chat_completions。"
            },
        ]
    }

    static func render(into directory: URL, width: CGFloat = 880) async throws {
        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true
        )
        for shot in shots() {
            let model = RouterPanelModel(repository: shot.repository)
            await model.load(range: DashboardRangeV1(startAt: 0, endAt: 1))
            await shot.mutate(model)

            let settings = NativeAppearanceSettings(
                lightThemeId: shot.appearance == .dark ? "system" : shot.themeId,
                darkThemeId: shot.appearance == .dark ? shot.themeId : "system",
                // 材质在无头渲染下背后没东西可采样,一律渲不透明。
                translucencyEnabled: false
            )
            let resolved = settings.resolve(
                appearance: shot.appearance, systemReduceTransparency: false
            )

            // 渲 `.content` 而不是整个视图 —— ImageRenderer 渲不出 ScrollView 里的东西。
            let panel = RouterPanelView(model: model, appearance: resolved)
            let view = panel.content
                .frame(width: width)
                .background(resolved.theme.ground)
                .fixedSize(horizontal: false, vertical: true)
                .environment(\.colorScheme, shot.appearance == .dark ? .dark : .light)

            let renderer = ImageRenderer(content: view)
            renderer.scale = 2
            guard let image = renderer.nsImage,
                  let tiff = image.tiffRepresentation,
                  let rep = NSBitmapImageRep(data: tiff),
                  let png = rep.representation(using: .png, properties: [:])
            else {
                FileHandle.standardError.write(Data("渲染失败: \(shot.name)\n".utf8))
                continue
            }
            let url = directory.appendingPathComponent("\(shot.name).png")
            try png.write(to: url)
            print("\(url.path)  \(Int(image.size.width))x\(Int(image.size.height))")
        }
    }
}

/// 只有显式给了 `PANEL_SHOTS_DIR` 才渲图 —— 平时跑测试不该往磁盘写文件。
@MainActor
@Test func renderPanelSnapshots() async throws {
    guard let path = ProcessInfo.processInfo.environment["PANEL_SHOTS_DIR"],
          !path.isEmpty
    else { return }
    try await PanelRenderer.render(into: URL(fileURLWithPath: path))
}
