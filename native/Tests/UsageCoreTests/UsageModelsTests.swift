import Foundation
import Testing
@testable import UsageCore

@Test func remainingPercentageIsClamped() {
    let exhausted = QuotaWindow(
        id: "exhausted", title: "Window", utilizationPercent: 140, resetsAt: nil, health: .critical
    )
    let fresh = QuotaWindow(
        id: "fresh", title: "Window", utilizationPercent: -10, resetsAt: nil, health: .healthy
    )

    #expect(exhausted.remainingPercent == 0)
    #expect(fresh.remainingPercent == 100)
}

@Test func snapshotUsesMostUrgentHealth() {
    let healthy = ProviderUsage(
        id: "healthy",
        name: "Healthy",
        windows: [.init(id: "daily", title: "Daily", utilizationPercent: 10, resetsAt: nil, health: .healthy)]
    )
    let critical = ProviderUsage(
        id: "critical",
        name: "Critical",
        windows: [.init(id: "weekly", title: "Weekly", utilizationPercent: 99, resetsAt: nil, health: .critical)]
    )

    #expect(UsageSnapshot(generatedAt: .now, providers: [healthy, critical]).health == .critical)
}

@Test func snapshotStoreRoundTrips() async throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = UsageSnapshotStore(fileURL: directory.appendingPathComponent("snapshot.json"))
    let snapshot = UsageSnapshot(
        generatedAt: Date(timeIntervalSince1970: 1_700_000_000),
        providers: [.init(id: "codex", name: "Codex", windows: [])]
    )

    try await store.save(snapshot)

    #expect(try await store.load() == snapshot)
}
