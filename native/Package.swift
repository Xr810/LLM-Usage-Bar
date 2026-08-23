// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "LLMUsageBarNative",
    platforms: [.macOS(.v13)],
    products: [
        // T27:可执行 target 暂时移除 —— 14 个旧视图没有随 bridge 移栽过来,
        // 新视图按 docs/design/2026-08-23-router-panel-visual-direction.md 重写后再加回。
        .library(name: "UsageCore", targets: ["UsageCore"]),
    ],
    targets: [
        .target(
            name: "UsageCore",
            swiftSettings: [
                .define("NATIVE_PREVIEW_SUPPORT", .when(configuration: .debug)),
            ]
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
