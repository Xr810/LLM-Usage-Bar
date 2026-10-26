import Foundation

public struct DashboardRangeV1: Equatable, Sendable {
    public var startAt: Int64
    public var endAt: Int64

    public init(startAt: Int64, endAt: Int64) {
        self.startAt = startAt
        self.endAt = endAt
    }
}

public protocol DashboardRepository: Sendable {
    func providerDashboard(range: DashboardRangeV1) async throws -> ProviderMonitoringDashboardV1
    func providerActivity(range: DashboardRangeV1) async throws -> [DashboardTrendBucketV1]
    func modelDashboard(range: DashboardRangeV1) async throws -> ModelUsageDashboardV1
    func agentBreakdown(range: DashboardRangeV1) async throws -> AgentUsageBreakdownV1
    func usageEvents(
        providerId: String,
        range: DashboardRangeV1,
        page: UInt64,
        pageSize: UInt64
    ) async throws -> UsageEventPageV1
}

public actor BridgeDashboardRepository: DashboardRepository {
    public static let supportedSchemaVersion = 1
    private let client: NativeBridgeClient

    public init(client: NativeBridgeClient = NativeBridgeClient()) {
        self.client = client
    }

    public func providerDashboard(
        range: DashboardRangeV1
    ) async throws -> ProviderMonitoringDashboardV1 {
        let envelope: BridgeSchemaEnvelopeV1<ProviderMonitoringDashboardV1> = try await client.call(
            "getProviderDashboard",
            params: rangeParameters(range),
            as: BridgeSchemaEnvelopeV1<ProviderMonitoringDashboardV1>.self
        )
        return try Self.unwrap(envelope)
    }

    public func modelDashboard(range: DashboardRangeV1) async throws -> ModelUsageDashboardV1 {
        let envelope: BridgeSchemaEnvelopeV1<ModelUsageDashboardV1> = try await client.call(
            "getModelDashboard",
            params: rangeParameters(range),
            as: BridgeSchemaEnvelopeV1<ModelUsageDashboardV1>.self
        )
        return try Self.unwrap(envelope)
    }

    public func providerActivity(
        range: DashboardRangeV1
    ) async throws -> [DashboardTrendBucketV1] {
        let envelope: BridgeSchemaEnvelopeV1<[DashboardTrendBucketV1]> = try await client.call(
            "getProviderUsageActivity",
            params: rangeParameters(range),
            as: BridgeSchemaEnvelopeV1<[DashboardTrendBucketV1]>.self
        )
        return try Self.unwrap(envelope)
    }

    public func agentBreakdown(range: DashboardRangeV1) async throws -> AgentUsageBreakdownV1 {
        let envelope: BridgeSchemaEnvelopeV1<AgentUsageBreakdownV1> = try await client.call(
            "getAgentBreakdown",
            params: rangeParameters(range),
            as: BridgeSchemaEnvelopeV1<AgentUsageBreakdownV1>.self
        )
        return try Self.unwrap(envelope)
    }

    public func usageEvents(
        providerId: String,
        range: DashboardRangeV1,
        page: UInt64,
        pageSize: UInt64
    ) async throws -> UsageEventPageV1 {
        let envelope: BridgeSchemaEnvelopeV1<UsageEventPageV1> = try await client.call(
            "getUsageEvents",
            params: [
                "providerId": .string(providerId),
                "startAt": .integer(range.startAt),
                "endAt": .integer(range.endAt),
                "page": .unsigned(page),
                "pageSize": .unsigned(pageSize),
            ],
            as: BridgeSchemaEnvelopeV1<UsageEventPageV1>.self
        )
        return try Self.unwrap(envelope)
    }

    private func rangeParameters(_ range: DashboardRangeV1) -> [String: NativeBridgeParameter] {
        [
            "startAt": .integer(range.startAt),
            "endAt": .integer(range.endAt),
        ]
    }

    nonisolated static func unwrap<Value: Codable & Sendable>(
        _ envelope: BridgeSchemaEnvelopeV1<Value>
    ) throws -> Value {
        guard envelope.schemaVersion == Self.supportedSchemaVersion else {
            throw UsageRepositoryError.unsupportedDashboardSchema(envelope.schemaVersion)
        }
        return envelope.data
    }
}
