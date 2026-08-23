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
