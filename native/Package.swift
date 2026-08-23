// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "LLMUsageBarNative",
    platforms: [.macOS(.v13)],
    products: [
        // T27:可执行 target 暂时移除 —— 14 个旧视图没有随 bridge 移栽过来,
        // 新视图按 docs/design/2026-08-23-router-panel-visual-direction.md 重写后再加回。
        .library(name: "UsageCore", targets: ["UsageCore"]),
        .library(name: "NativeUI", targets: ["NativeUI"]),
        // 只为「把界面跑起来看一眼」存在,不是产品外壳。产品外壳(菜单栏、窗口、
        // 生命周期)是另一件事,归后面的任务。
        .executable(name: "RouterPanelPreview", targets: ["RouterPanelPreview"]),
    ],
    targets: [
        .target(
            name: "UsageCore",
            swiftSettings: [
                .define("NATIVE_PREVIEW_SUPPORT", .when(configuration: .debug)),
            ]
        ),
        // T29:设计系统单独成 target —— UsageCore 是数据与传输层,不该被 SwiftUI 拖进去。
        // 视图将来长在可执行 target 里,依赖这一层拿 token。
        .target(name: "NativeUI", dependencies: ["UsageCore"]),
        .executableTarget(name: "RouterPanelPreview", dependencies: ["NativeUI", "UsageCore"]),
        .testTarget(
            name: "NativeUITests",
            dependencies: ["NativeUI", "UsageCore"]
        ),
        .testTarget(
            name: "UsageCoreTests",
            dependencies: ["UsageCore"],
            resources: [.copy("Fixtures")],
            swiftSettings: [
                .define("NATIVE_PREVIEW_SUPPORT", .when(configuration: .debug)),
            ]
        ),
    ]
)
