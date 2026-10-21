import Foundation

public struct BridgeSchemaEnvelopeV1<Value: Codable & Sendable>: Codable, Sendable {
    public var schemaVersion: Int
    public var data: Value
}

public enum UsageTrendGranularityV1: String, Codable, Sendable {
    case hour
    case day
}

public enum TokenSourceV1: String, Codable, Sendable {
    case proxy
    case sessionLog = "session_log"
}

public enum CostSourceV1: String, Codable, Sendable {
    case upstream
    case estimated
    case unavailable
}

public enum PricingOriginV1: String, Codable, Sendable {
    case user
    case official
}

/// The stable account identity needed by native read-only surfaces.
/// Rust deliberately projects only these fields so credentials, bindings, and
/// route details never cross the bridge or enter UI state.
public struct UsageProviderSummaryV1: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var billingKind: BillingKind
    public var productGroupId: String
    public var enabled: Bool
    public var systemPresetKey: String?
}

public struct QuotaWindowPaceV1: Codable, Equatable, Sendable {
    public var status: TrayUsageStatus
    public var burnRatePercentPerHour: String?
    public var projectedExhaustAt: String?
    public var headroomRatio: String?
    public var paceBasis: PaceBasis
    public var rhythmAdjustment: String?
    public var flatStatus: TrayUsageStatus?

    public var burnRatePerHour: Decimal? { bridgeDecimal(burnRatePercentPerHour) }
}

public struct QuotaStatusV1: Codable, Equatable, Sendable {
    public var snapshotId: String
    public var fetchedAt: Int64
    public var sourceObservedAt: Int64?
    public var planType: String?
    public var planRenewsAt: Int64?
    public var fiveHourUtilizationPercent: String?
    public var fiveHourResetsAt: String?
    public var sevenDayUtilizationPercent: String?
    public var sevenDayResetsAt: String?
    public var fiveHourPace: QuotaWindowPaceV1
    public var sevenDayPace: QuotaWindowPaceV1
    public var manualResetsRemaining: Int64?
    public var manualResetCredits: [ManualResetCreditV1]

    public var fiveHourRemainingPercent: Decimal? {
        bridgeDecimal(fiveHourUtilizationPercent).map { max(0, 100 - $0) }
    }

    public var sevenDayRemainingPercent: Decimal? {
        bridgeDecimal(sevenDayUtilizationPercent).map { max(0, 100 - $0) }
    }
}

public struct QuotaFetchStateV1: Codable, Equatable, Sendable {
    public var providerId: String
    public var lastAttemptAt: Int64?
    public var lastSuccessAt: Int64?
    public var lastError: String?
    public var consecutiveFailures: UInt32
    public var stale: Bool
}

public struct ProviderDashboardRowV1: Codable, Equatable, Identifiable, Sendable {
    public var provider: UsageProviderSummaryV1
    public var sharedAccount: Bool
    public var eventCount: UInt64
    public var inputTokens: UInt64
    public var outputTokens: UInt64
    public var cacheReadTokens: UInt64
    public var cacheCreationTokens: UInt64
    public var totalCostUsd: String?
    public var costSourceCounts: CostSourceCountsV1
    public var quota: QuotaStatusV1?
    public var quotaFetchState: QuotaFetchStateV1?
    /// Safe presentation metadata. Optional so a new native client can still
    /// decode dashboard responses from an older development bridge.
    public var tokenSources: [TokenSourceV1]?

    public var id: String { provider.id }
    public var totalTokens: UInt64 {
        inputTokens + outputTokens + cacheReadTokens + cacheCreationTokens
    }
    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct DashboardTrendBucketV1: Codable, Equatable, Identifiable, Sendable {
    public var startAt: Int64
    public var endAt: Int64
    public var eventCount: UInt64
    public var inputTokens: UInt64
    public var outputTokens: UInt64
    public var cacheReadTokens: UInt64
    public var cacheCreationTokens: UInt64
    public var totalTokens: UInt64
    public var totalCostUsd: String?
    public var costSourceCounts: CostSourceCountsV1

    public var id: Int64 { startAt }
    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct ProviderMonitoringDashboardV1: Codable, Equatable, Sendable {
    public var startAt: Int64
    public var endAt: Int64
    public var providers: [ProviderDashboardRowV1]
    public var trendGranularity: UsageTrendGranularityV1
    public var trendBuckets: [DashboardTrendBucketV1]
    public var warnings: [String]

    public static let empty = Self(
        startAt: 0,
        endAt: 0,
        providers: [],
        trendGranularity: .day,
        trendBuckets: [],
        warnings: []
    )
}

public struct ModelUsageRowV1: Codable, Equatable, Identifiable, Sendable {
    public var model: String
    public var providerId: String
    public var providerName: String
    public var productGroupId: String
    public var billingKind: BillingKind
    public var eventCount: UInt64
    public var inputTokens: UInt64
    public var outputTokens: UInt64
    public var cacheReadTokens: UInt64
    public var cacheCreationTokens: UInt64
    public var totalTokens: UInt64
    public var totalCostUsd: String?
    public var costSourceCounts: CostSourceCountsV1
    public var firstOccurredAt: Int64
    public var lastOccurredAt: Int64

    public var id: String { "\(providerId):\(model)" }
    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct ModelTotalsRowV1: Codable, Equatable, Identifiable, Sendable {
    public var model: String
    public var providerIds: [String]
    public var eventCount: UInt64
    public var inputTokens: UInt64
    public var outputTokens: UInt64
    public var cacheReadTokens: UInt64
    public var cacheCreationTokens: UInt64
    public var totalTokens: UInt64
    public var totalCostUsd: String?
    public var costSourceCounts: CostSourceCountsV1
    public var firstOccurredAt: Int64
    public var lastOccurredAt: Int64

    public var id: String { model }
    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct ModelProductGroupV1: Codable, Equatable, Identifiable, Sendable {
    public var productGroupId: String
    public var billingKind: BillingKind
    public var providerIds: [String]
    public var providerNames: [String]
    public var eventCount: UInt64
    public var inputTokens: UInt64
    public var outputTokens: UInt64
    public var cacheReadTokens: UInt64
    public var cacheCreationTokens: UInt64
    public var totalTokens: UInt64
    public var totalCostUsd: String?
    public var costSourceCounts: CostSourceCountsV1
    public var models: [ModelUsageRowV1]

    public var id: String { productGroupId }
    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct ModelUsageDashboardV1: Codable, Equatable, Sendable {
    public var startAt: Int64
    public var endAt: Int64
    public var totalTokens: UInt64
    public var totalEventCount: UInt64
    public var totalCostUsd: String?
    public var productGroups: [ModelProductGroupV1]
    public var models: [ModelTotalsRowV1]
    public var warnings: [String]

    public static let empty = Self(
        startAt: 0,
        endAt: 0,
        totalTokens: 0,
        totalEventCount: 0,
        totalCostUsd: nil,
        productGroups: [],
        models: [],
        warnings: []
    )

    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct AgentProviderUsageRowV1: Codable, Equatable, Identifiable, Sendable {
    public var providerId: String
    public var providerName: String
    public var productGroupId: String
    public var billingKind: BillingKind
    public var eventCount: UInt64
    public var inputTokens: UInt64
    public var outputTokens: UInt64
    public var cacheReadTokens: UInt64
    public var cacheCreationTokens: UInt64
    public var totalTokens: UInt64
    public var totalCostUsd: String?
    public var costSourceCounts: CostSourceCountsV1

    public var id: String { providerId }
    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct AgentUsageRowV1: Codable, Equatable, Identifiable, Sendable {
    public var agentModuleId: String?
    public var agentName: String?
    public var archived: Bool
    public var visible: Bool
    public var eventCount: UInt64
    public var inputTokens: UInt64
    public var outputTokens: UInt64
    public var cacheReadTokens: UInt64
    public var cacheCreationTokens: UInt64
    public var totalTokens: UInt64
    public var totalCostUsd: String?
    public var costSourceCounts: CostSourceCountsV1
    public var firstOccurredAt: Int64
    public var lastOccurredAt: Int64
    public var providers: [AgentProviderUsageRowV1]
    public var models: [ModelTotalsRowV1]

    public var id: String { agentModuleId ?? "unassigned" }
    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct AgentUsageBreakdownV1: Codable, Equatable, Sendable {
    public var startAt: Int64
    public var endAt: Int64
    public var totalTokens: UInt64
    public var totalEventCount: UInt64
    public var totalCostUsd: String?
    public var agents: [AgentUsageRowV1]
    public var warnings: [String]

    public static let empty = Self(
        startAt: 0,
        endAt: 0,
        totalTokens: 0,
        totalEventCount: 0,
        totalCostUsd: nil,
        agents: [],
        warnings: []
    )

    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct UsageEventV1: Codable, Equatable, Identifiable, Sendable {
    public var eventId: String
    public var source: TokenSourceV1
    public var providerId: String
    public var agentModuleId: String?
    public var productGroupId: String
    public var occurredAt: Int64
    public var model: String
    public var inputTokens: UInt64
    public var outputTokens: UInt64
    public var cacheReadTokens: UInt64
    public var cacheCreationTokens: UInt64
    public var requestId: String?
    public var sessionId: String?
    public var upstreamCorrelationId: String?
    public var inputCostUsd: String?
    public var outputCostUsd: String?
    public var cacheReadCostUsd: String?
    public var cacheCreationCostUsd: String?
    public var totalCostUsd: String?
    public var costSource: CostSourceV1
    public var pricingOrigin: PricingOriginV1?
    public var legacyRequestId: String?
    public var createdAt: Int64

    public var id: String { eventId }
    public var totalTokens: UInt64 {
        inputTokens + outputTokens + cacheReadTokens + cacheCreationTokens
    }
    public var totalCost: Decimal? { bridgeDecimal(totalCostUsd) }
}

public struct UsageEventPageV1: Codable, Equatable, Sendable {
    public var items: [UsageEventV1]
    public var total: UInt64
    public var page: UInt64
    public var pageSize: UInt64

    public static let empty = Self(items: [], total: 0, page: 1, pageSize: 50)
}

private func bridgeDecimal(_ value: String?) -> Decimal? {
    value.flatMap { Decimal(string: $0, locale: Locale(identifier: "en_US_POSIX")) }
}
