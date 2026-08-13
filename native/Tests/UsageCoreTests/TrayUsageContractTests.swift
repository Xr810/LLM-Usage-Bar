import Foundation
import Testing
@testable import UsageCore

@Test func rustTrayFixtureDecodesWithoutLosingUnknownState() throws {
    let data = try Data(contentsOf: Bundle.module.url(
        forResource: "tray-usage-snapshot-v1",
        withExtension: "json",
        subdirectory: "Fixtures"
    )!)
    let snapshot = try JSONDecoder().decode(TrayUsageSnapshotV1.self, from: data)

    #expect(snapshot.status == .yellow)
    #expect(snapshot.providers.count == 2)
    #expect(snapshot.providers[0].subscription?.windows[1].remaining == nil)
    #expect(snapshot.providers[0].subscription?.windows[1].status == .unknown)
    #expect(snapshot.apiBudget.todayCost == Decimal(string: "4.25"))
}

@Test func persistedSnapshotRejectsUnknownSchema() async throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let file = directory.appendingPathComponent("tray-snapshot-v1.json")
    try Data("{\"protocolVersion\":1,\"schemaVersion\":99,\"writtenAt\":0,\"snapshot\":{}}".utf8)
        .write(to: file)
    let repository = SnapshotFileUsageRepository(fileURL: file)

    await #expect(throws: UsageRepositoryError.unsupportedSnapshotSchema(99)) {
        try await repository.snapshot()
    }
}

@Test func sharedDashboardFixturePreservesProviderModelAgentAndEventSemantics() throws {
    struct Fixture: Decodable {
        let providerDashboard: BridgeSchemaEnvelopeV1<ProviderMonitoringDashboardV1>
        let modelDashboard: BridgeSchemaEnvelopeV1<ModelUsageDashboardV1>
        let agentBreakdown: BridgeSchemaEnvelopeV1<AgentUsageBreakdownV1>
        let usageEvents: BridgeSchemaEnvelopeV1<UsageEventPageV1>
    }

    let data = try Data(contentsOf: Bundle.module.url(
        forResource: "dashboard-contract-v1",
        withExtension: "json",
        subdirectory: "Fixtures"
    )!)
    let fixture = try JSONDecoder().decode(Fixture.self, from: data)

    #expect(fixture.providerDashboard.schemaVersion == 1)
    #expect(fixture.providerDashboard.data.providers[0].id == "codex-work")
    #expect(fixture.providerDashboard.data.providers[0].totalCost == nil)
    #expect(fixture.providerDashboard.data.providers[0].quota?.sevenDayRemainingPercent == nil)
    #expect(fixture.modelDashboard.data.totalTokens == 2_600)
    #expect(fixture.agentBreakdown.data.agents[0].agentModuleId == "codex")
    #expect(fixture.usageEvents.data.items[0].totalCost == nil)
}

@Test func dashboardRepositoryRejectsUnknownSchema() throws {
    let envelope = BridgeSchemaEnvelopeV1(
        schemaVersion: 99,
        data: ProviderMonitoringDashboardV1.empty
    )
    #expect(throws: UsageRepositoryError.unsupportedDashboardSchema(99)) {
        try BridgeDashboardRepository.unwrap(envelope)
    }
}
