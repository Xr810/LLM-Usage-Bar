// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "LLMUsageBarNative",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "LLMUsageBarNative", targets: ["LLMUsageBarNative"]),
        .library(name: "UsageCore", targets: ["UsageCore"]),
    ],
    targets: [
        .target(name: "UsageCore"),
        .executableTarget(
            name: "LLMUsageBarNative",
            dependencies: ["UsageCore"]
        ),
        .testTarget(name: "UsageCoreTests", dependencies: ["UsageCore"]),
    ]
)
