import Foundation

public protocol UsageRepository: Sendable {
    func snapshot() async throws -> TrayUsageSnapshotV1
    func refresh() async throws -> TrayUsageSnapshotV1
    func runtimeStatus() async throws -> BridgeRuntimeStatusV1
    func shutdownBridge(destination: LegacyHandoffDestinationV1?) async -> Bool
    func disconnect() async
}

public enum LegacyHandoffDestinationV1: String, Codable, Sendable {
    case usage
    case settings
}

public protocol CredentialRepository: Sendable {}

public struct BridgeRuntimeStatusV1: Codable, Equatable, Sendable {
    public var bridgeOnly: Bool
    public var clientCount: Int
    public var databaseOwner: String
    public var schedulerOwner: String
}

public enum UsageRepositoryError: Error, Equatable, Sendable {
    case bridgeUnavailable
    case unsupportedProtocol(Int)
    case unsupportedSnapshotSchema(Int)
    case unsupportedDashboardSchema(Int)
    case mutationUnavailable(schemaVersion: Int?)
    case malformedResponse
    case remote(code: String, message: String, data: NativeBridgeParameter?)
    case requestTooLarge
}

public enum NativeApiBudgetModeV1: String, Codable, Equatable, Sendable {
    case shared
    case perProvider
}

public struct NativeSettingsV1: Codable, Equatable, Sendable {
    public var launchOnStartup: Bool
    public var silentStartup: Bool
    public var showInTray: Bool
    public var minimizeToTrayOnClose: Bool
    public var language: String?
    public var usageWarningRemainingPercent: UInt8
    public var usageCriticalRemainingPercent: UInt8
    public var apiBudgetMode: NativeApiBudgetModeV1
    public var sharedApiDailyBudgetUsd: String?
    public var usageDashboardRefreshIntervalMs: UInt32?

    public init(
        launchOnStartup: Bool,
        silentStartup: Bool,
        showInTray: Bool,
        minimizeToTrayOnClose: Bool,
        language: String?,
        usageWarningRemainingPercent: UInt8,
        usageCriticalRemainingPercent: UInt8,
        apiBudgetMode: NativeApiBudgetModeV1,
        sharedApiDailyBudgetUsd: String?,
        usageDashboardRefreshIntervalMs: UInt32?
    ) {
        self.launchOnStartup = launchOnStartup
        self.silentStartup = silentStartup
        self.showInTray = showInTray
        self.minimizeToTrayOnClose = minimizeToTrayOnClose
        self.language = language
        self.usageWarningRemainingPercent = usageWarningRemainingPercent
        self.usageCriticalRemainingPercent = usageCriticalRemainingPercent
        self.apiBudgetMode = apiBudgetMode
        self.sharedApiDailyBudgetUsd = sharedApiDailyBudgetUsd
        self.usageDashboardRefreshIntervalMs = usageDashboardRefreshIntervalMs
    }
}

public struct NativeSettingsDocumentV1: Codable, Equatable, Sendable {
    public var schemaVersion: Int
    public var data: NativeSettingsV1
    public var revision: String

    public init(schemaVersion: Int, data: NativeSettingsV1, revision: String) {
        self.schemaVersion = schemaVersion
        self.data = data
        self.revision = revision
    }
}

public enum NativeSettingsPatchValueV1: Equatable, Sendable {
    case launchOnStartup(Bool)
    case silentStartup(Bool)
    case showInTray(Bool)
    case minimizeToTrayOnClose(Bool)
    case language(String?)
    case usageWarningRemainingPercent(UInt8)
    case usageCriticalRemainingPercent(UInt8)
    case apiBudgetMode(NativeApiBudgetModeV1)
    case sharedApiDailyBudgetUsd(String?)
    case usageDashboardRefreshIntervalMs(UInt32?)
}

public struct NativeSettingsPatchV1: Equatable, Sendable {
    public var values: [NativeSettingsPatchValueV1]

    public init(_ values: [NativeSettingsPatchValueV1] = []) {
        self.values = values
    }

    var bridgeValue: NativeBridgeParameter {
        var object: [String: NativeBridgeParameter] = [:]
        for value in values {
            switch value {
            case let .launchOnStartup(value):
                object["launchOnStartup"] = .boolean(value)
            case let .silentStartup(value):
                object["silentStartup"] = .boolean(value)
            case let .showInTray(value):
                object["showInTray"] = .boolean(value)
            case let .minimizeToTrayOnClose(value):
                object["minimizeToTrayOnClose"] = .boolean(value)
            case let .language(value):
                object["language"] = value.map(NativeBridgeParameter.string) ?? .null
            case let .usageWarningRemainingPercent(value):
                object["usageWarningRemainingPercent"] = .unsigned(UInt64(value))
            case let .usageCriticalRemainingPercent(value):
                object["usageCriticalRemainingPercent"] = .unsigned(UInt64(value))
            case let .apiBudgetMode(value):
                object["apiBudgetMode"] = .string(value.rawValue)
            case let .sharedApiDailyBudgetUsd(value):
                object["sharedApiDailyBudgetUsd"] = value.map(NativeBridgeParameter.string) ?? .null
            case let .usageDashboardRefreshIntervalMs(value):
                object["usageDashboardRefreshIntervalMs"] = value
                    .map { .unsigned(UInt64($0)) } ?? .null
            }
        }
        return .object(object)
    }
}

public protocol SettingsRepository: Sendable {
    func load() async throws -> NativeSettingsDocumentV1
    func save(
        patch: NativeSettingsPatchV1,
        expectedRevision: String
    ) async throws -> NativeSettingsDocumentV1
}

public enum SettingsRepositoryError: Error, Equatable, Sendable {
    case conflict(current: NativeSettingsDocumentV1)
    case mutationUnavailable(schemaVersion: Int?)
    case unsupportedSchema(Int)
    case remote(code: String, message: String, key: String?)
    case transport(UsageRepositoryError)
}

public actor BridgeSettingsRepository: SettingsRepository {
    public static let supportedSchemaVersion = 1
    private let client: NativeBridgeClient

    public init(client: NativeBridgeClient = NativeBridgeClient()) {
        self.client = client
    }

    public func load() async throws -> NativeSettingsDocumentV1 {
        do {
            let document: NativeSettingsDocumentV1 = try await client.call(
                "getNativeSettings",
                as: NativeSettingsDocumentV1.self
            )
            return try Self.unwrap(document)
        } catch {
            throw Self.map(error)
        }
    }

    public func save(
        patch: NativeSettingsPatchV1,
        expectedRevision: String
    ) async throws -> NativeSettingsDocumentV1 {
        do {
            let document: NativeSettingsDocumentV1 = try await client.callMutation(
                "setNativeSettings",
                params: [
                    "expectedRevision": .string(expectedRevision),
                    "patch": patch.bridgeValue,
                ],
                as: NativeSettingsDocumentV1.self
            )
            return try Self.unwrap(document)
        } catch {
            throw Self.map(error)
        }
    }

    static func unwrap(_ document: NativeSettingsDocumentV1) throws -> NativeSettingsDocumentV1 {
        guard document.schemaVersion == Self.supportedSchemaVersion else {
            throw SettingsRepositoryError.unsupportedSchema(document.schemaVersion)
        }
        return document
    }

    static func map(_ error: Error) -> SettingsRepositoryError {
        if let error = error as? SettingsRepositoryError {
            return error
        }
        guard let error = error as? UsageRepositoryError else {
            return .transport(.malformedResponse)
        }
        switch error {
        case let .mutationUnavailable(schemaVersion):
            return .mutationUnavailable(schemaVersion: schemaVersion)
        case let .remote(code, message, data):
            if code == "settings_conflict",
               let document = decodeConflict(data),
               document.schemaVersion == Self.supportedSchemaVersion {
                return .conflict(current: document)
            }
            return .remote(code: code, message: message, key: settingKey(data))
        default:
            return .transport(error)
        }
    }

    private static func decodeConflict(
        _ data: NativeBridgeParameter?
    ) -> NativeSettingsDocumentV1? {
        guard let data,
              let encoded = try? JSONEncoder().encode(data) else { return nil }
        return try? JSONDecoder().decode(NativeSettingsDocumentV1.self, from: encoded)
    }

    private static func settingKey(_ data: NativeBridgeParameter?) -> String? {
        guard case let .object(object) = data,
              case let .string(key) = object["key"] else { return nil }
        return key
    }
}

public actor BridgeUsageRepository: UsageRepository {
    private let client: NativeBridgeClient

    public init(client: NativeBridgeClient = NativeBridgeClient()) {
        self.client = client
    }

    public func snapshot() async throws -> TrayUsageSnapshotV1 {
        try await client.call("getTrayUsageSnapshot", as: TrayUsageSnapshotV1.self)
    }

    public func refresh() async throws -> TrayUsageSnapshotV1 {
        try await client.call("refreshTrayUsage", as: TrayUsageSnapshotV1.self)
    }

    public func runtimeStatus() async throws -> BridgeRuntimeStatusV1 {
        try await client.call("getRuntimeStatus", as: BridgeRuntimeStatusV1.self)
    }

    public func shutdownBridge(destination: LegacyHandoffDestinationV1?) async -> Bool {
        let params = destination.map { ["destination": NativeBridgeParameter.string($0.rawValue)] }
        let response = try? await client.call("shutdown", params: params, as: ShutdownResponse.self)
        await client.disconnect()
        return response?.accepted == true
    }

    public func disconnect() async {
        await client.disconnect()
    }

    private struct ShutdownResponse: Decodable, Sendable {
        let accepted: Bool
    }
}

public actor SnapshotFileUsageRepository: UsageRepository {
    private struct Envelope: Decodable {
        let protocolVersion: Int
        let schemaVersion: Int
        let writtenAt: Int64
        var snapshot: TrayUsageSnapshotV1
    }

    private struct EnvelopeHeader: Decodable {
        let protocolVersion: Int
        let schemaVersion: Int
    }

    public static let supportedSchemaVersion = 1
    public let fileURL: URL

    public init(fileURL: URL = NativeBridgeClient.defaultSnapshotURL()) {
        self.fileURL = fileURL
    }

    public func snapshot() throws -> TrayUsageSnapshotV1 {
        let data = try Data(contentsOf: fileURL)
        let decoder = JSONDecoder()
        let header = try decoder.decode(EnvelopeHeader.self, from: data)
        guard header.protocolVersion == NativeBridgeClient.protocolVersion else {
            throw UsageRepositoryError.unsupportedProtocol(header.protocolVersion)
        }
        guard header.schemaVersion == Self.supportedSchemaVersion else {
            throw UsageRepositoryError.unsupportedSnapshotSchema(header.schemaVersion)
        }
        let envelope = try decoder.decode(Envelope.self, from: data)
        var snapshot = envelope.snapshot
        snapshot.stale = true
        return snapshot
    }

    public func refresh() throws -> TrayUsageSnapshotV1 { try snapshot() }

    public func runtimeStatus() throws -> BridgeRuntimeStatusV1 {
        throw UsageRepositoryError.bridgeUnavailable
    }

    public func shutdownBridge(destination: LegacyHandoffDestinationV1?) async -> Bool { false }
    public func disconnect() async {}
}

public actor ResilientUsageRepository: UsageRepository {
    private let bridge: BridgeUsageRepository
    private let fallback: SnapshotFileUsageRepository

    public init(
        bridge: BridgeUsageRepository = BridgeUsageRepository(),
        fallback: SnapshotFileUsageRepository = SnapshotFileUsageRepository()
    ) {
        self.bridge = bridge
        self.fallback = fallback
    }

    public func snapshot() async throws -> TrayUsageSnapshotV1 {
        do { return try await bridge.snapshot() }
        catch { return try await fallback.snapshot() }
    }

    public func refresh() async throws -> TrayUsageSnapshotV1 {
        do { return try await bridge.refresh() }
        catch { return try await fallback.snapshot() }
    }

    public func runtimeStatus() async throws -> BridgeRuntimeStatusV1 {
        try await bridge.runtimeStatus()
    }

    public func shutdownBridge(destination: LegacyHandoffDestinationV1?) async -> Bool {
        await bridge.shutdownBridge(destination: destination)
    }

    public func disconnect() async {
        await bridge.disconnect()
    }
}
