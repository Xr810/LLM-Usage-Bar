import Foundation
import Testing
@testable import UsageCore

@Test func statuslineSourceLoadsNewestCurrentWindows() throws {
    let now = Date(timeIntervalSince1970: 1_700_000_000)
    let fileURL = try fixtureFile(
        """
        {
          "schemaVersion": 5,
          "updatedAt": 1700000000,
          "sessions": [
            {"rateLimits":{"fiveHour":{"usedPercentage":20,"resetsAt":1700003600,"observedAt":1699999900}}},
            {"rateLimits":{"fiveHour":{"usedPercentage":82,"resetsAt":1700007200,"observedAt":1700000000},"sevenDay":{"usedPercentage":95,"resetsAt":1700600000,"observedAt":1700000000}}}
          ]
        }
        """
    )
    defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

    let provider = try ClaudeStatuslineSource(fileURL: fileURL).load(now: now)

    #expect(provider.windows.map(\.utilizationPercent) == [82, 95])
    #expect(provider.windows.map(\.health) == [.warning, .critical])
    #expect(provider.health == .critical)
}

@Test func statuslineSourceRejectsStaleCache() throws {
    let fileURL = try fixtureFile(
        """
        {"schemaVersion":5,"updatedAt":100,"sessions":[]}
        """
    )
    defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

    #expect(throws: ClaudeStatuslineSource.SourceError.staleCache) {
        try ClaudeStatuslineSource(fileURL: fileURL).load(now: Date(timeIntervalSince1970: 2_000))
    }
}

@Test func statuslineSourceUsesProductionCacheLocation() {
    let home = URL(fileURLWithPath: "/Users/example", isDirectory: true)

    #expect(
        ClaudeStatuslineSource.defaultFileURL(homeDirectory: home).path
            == "/Users/example/.llm-usage-bar/runtime/claude-statusline-quota.json"
    )
}

private func fixtureFile(_ contents: String) throws -> URL {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let fileURL = directory.appendingPathComponent("claude-statusline-quota.json")
    try Data(contents.utf8).write(to: fileURL)
    return fileURL
}
