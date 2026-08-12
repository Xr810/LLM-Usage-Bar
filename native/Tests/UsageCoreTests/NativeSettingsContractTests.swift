import Foundation
import Testing
@testable import UsageCore

private func nativeSettingsFixtureData() throws -> Data {
    try Data(contentsOf: Bundle.module.url(
        forResource: "native-settings-v1",
        withExtension: "json",
        subdirectory: "Fixtures"
    )!)
}

@Test func nativeSettingsFixtureDecodes() throws {
    let document = try JSONDecoder().decode(
        NativeSettingsDocumentV1.self,
        from: nativeSettingsFixtureData()
    )

    #expect(document.schemaVersion == 1)
    #expect(document.data.language == "zh-TW")
    #expect(document.data.apiBudgetMode == .perProvider)
    #expect(document.data.sharedApiDailyBudgetUsd == "12.50")
    #expect(document.data.usageDashboardRefreshIntervalMs == 30_000)
}

@Test func nativeSettingsDecodeIgnoresUnknownFields() throws {
    var root = try #require(
        JSONSerialization.jsonObject(with: nativeSettingsFixtureData()) as? [String: Any]
    )
    root["futureEnvelopeField"] = true
    var settings = try #require(root["data"] as? [String: Any])
    settings["futureSafeSetting"] = "new-value"
    root["data"] = settings

    let data = try JSONSerialization.data(withJSONObject: root)
    let document = try JSONDecoder().decode(NativeSettingsDocumentV1.self, from: data)
    #expect(document.data.language == "zh-TW")
}

@Test func settingsRepositoryRejectsUnknownSchema() throws {
    var document = try JSONDecoder().decode(
        NativeSettingsDocumentV1.self,
        from: nativeSettingsFixtureData()
    )
    document.schemaVersion = 99

    #expect(throws: SettingsRepositoryError.unsupportedSchema(99)) {
        try BridgeSettingsRepository.unwrap(document)
    }
}

@Test func settingsConflictCarriesFreshProjection() throws {
    let current = try JSONDecoder().decode(
        NativeSettingsDocumentV1.self,
        from: nativeSettingsFixtureData()
    )
    let data = try JSONEncoder().encode(current)
    let bridgeValue = try JSONDecoder().decode(NativeBridgeParameter.self, from: data)
    let mapped = BridgeSettingsRepository.map(
        UsageRepositoryError.remote(
            code: "settings_conflict",
            message: "Native settings changed concurrently",
            data: bridgeValue
        )
    )

    #expect(mapped == .conflict(current: current))
}

@Test func nativeSettingsPatchEncodesNestedObjectAndNull() throws {
    let patch = NativeSettingsPatchV1([
        .language(nil),
        .sharedApiDailyBudgetUsd("8.25"),
    ])
    let encoded = try JSONEncoder().encode([
        "patch": patch.bridgeValue,
    ])
    let root = try #require(
        JSONSerialization.jsonObject(with: encoded) as? [String: Any]
    )
    let object = try #require(root["patch"] as? [String: Any])
    #expect(object["language"] is NSNull)
    #expect(object["sharedApiDailyBudgetUsd"] as? String == "8.25")
}
