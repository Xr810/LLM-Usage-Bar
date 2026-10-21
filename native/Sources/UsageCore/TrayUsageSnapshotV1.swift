import Foundation

/// The stable Swift mirror of Rust's `TrayUsageSnapshot` DTO.
/// Decimal values remain strings at the transport boundary and are converted
/// to `Decimal` only by computed domain accessors.
public struct TrayUsageSnapshotV1: Codable, Equatable, Sendable {
    public var status: TrayUsageStatus
    public var generatedAt: Int64
    public var lastSuccessAt: Int64?
    public var stale: Bool
    public var refreshError: String?
    public var refreshInProgress: Bool
    public var apiBudget: TrayAPIBudgetV1
    public var agents: [TrayAgentUsageV1]

    public static func unavailable(at timestamp: Int64 = Int64(Date().timeIntervalSince1970)) -> Self {
        Self(
            status: .unknown,
            generatedAt: timestamp,
            lastSuccessAt: nil,
            stale: true,
            refreshError: "bridge_unavailable",
            refreshInProgress: false,
            apiBudget: .unavailable,
            agents: []
        )
    }

    public var providers: [TrayProviderUsageV1] {
        agents.flatMap(\.providers)
    }
}

public enum TrayUsageStatus: String, Codable, CaseIterable, Sendable {
    case green
    case yellow
    case red
    case unknown
}

public enum TrayCostQuality: String, Codable, Sendable {
    case complete
    case estimated
    case partial
    case unavailable
}

public enum PaceBasis: String, Codable, Sendable {
    case measured
    case windowAverage = "window_average"
    case `static`
    case idle
}

public enum BillingKind: String, Codable, Sendable {
    case subscription
    case metered
}

public enum APIBudgetMode: String, Codable, Sendable {
    case shared
    case perProvider = "per_provider"
}

public struct TrayAPIBudgetV1: Codable, Equatable, Sendable {
    public var mode: APIBudgetMode
    public var providerCount: Int
    public var todayCostUsd: String?
    public var dailyBudgetUsd: String?
    public var budgetConsumedPercent: String?
    public var costQuality: TrayCostQuality
    public var status: TrayUsageStatus
    public var warningReason: String?
    public var burnRateUsdPerHour: String?
    public var projectedExhaustAt: String?
    public var headroomRatio: String?
    public var paceBasis: PaceBasis
    public var rhythmAdjustment: String?
    public var flatStatus: TrayUsageStatus?

    public static let unavailable = Self(
        mode: .shared,
        providerCount: 0,
        todayCostUsd: nil,
        dailyBudgetUsd: nil,
        budgetConsumedPercent: nil,
        costQuality: .unavailable,
        status: .unknown,
        warningReason: "bridge_unavailable",
        burnRateUsdPerHour: nil,
        projectedExhaustAt: nil,
        headroomRatio: nil,
        paceBasis: .static,
        rhythmAdjustment: nil,
        flatStatus: nil
    )

    public var todayCost: Decimal? { decimal(todayCostUsd) }
    public var dailyBudget: Decimal? { decimal(dailyBudgetUsd) }
    public var consumedPercent: Decimal? { decimal(budgetConsumedPercent) }
    public var burnRatePerHour: Decimal? { decimal(burnRateUsdPerHour) }
}

public struct TrayAgentUsageV1: Codable, Equatable, Identifiable, Sendable {
    public var agentModuleId: String
    public var name: String
    public var sortOrder: Int64
    public var status: TrayUsageStatus
    public var providers: [TrayProviderUsageV1]

    public var id: String { agentModuleId }
}

public struct TrayProviderUsageV1: Codable, Equatable, Identifiable, Sendable {
    public var providerId: String
    public var providerName: String
    public var systemPresetKey: String?
    public var billingKind: BillingKind
    public var status: TrayUsageStatus
    public var warningReason: String?
    public var recentUsage: TrayProviderRecentUsageV1
    public var subscription: TraySubscriptionUsageV1?
    public var metered: TrayMeteredUsageV1?

    public var id: String { providerId }
}

public struct TrayProviderRecentUsageV1: Codable, Equatable, Sendable {
    public var startAt: Int64
    public var endAt: Int64
    public var todayTokens: UInt64
    public var totalTokens: UInt64
    public var todayCostUsd: String?
    public var totalCostUsd: String?
    public var costQuality: TrayCostQuality
    public var mostUsedModel: String?
    public var trendBuckets: [TrayUsageTrendBucketV1]

    public var todayCost: Decimal? { decimal(todayCostUsd) }
    public var totalCost: Decimal? { decimal(totalCostUsd) }
}

public struct TrayUsageTrendBucketV1: Codable, Equatable, Identifiable, Sendable {
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
    public var totalCost: Decimal? { decimal(totalCostUsd) }
}

public struct CostSourceCountsV1: Codable, Equatable, Sendable {
    public var upstream: UInt64
    public var estimated: UInt64
    public var unavailable: UInt64
}

public struct TrayQuotaWindowV1: Codable, Equatable, Identifiable, Sendable {
    public var kind: String
    public var usedPercent: String?
    public var remainingPercent: String?
    public var resetsAt: String?
    public var status: TrayUsageStatus
    public var unavailableReason: String?
    public var burnRatePercentPerHour: String?
    public var projectedExhaustAt: String?
    public var headroomRatio: String?
    public var paceBasis: PaceBasis
    public var rhythmAdjustment: String?
    public var flatStatus: TrayUsageStatus?

    public var id: String { kind }
    public var used: Decimal? { decimal(usedPercent) }
    public var remaining: Decimal? { decimal(remainingPercent) }
    public var burnRatePerHour: Decimal? { decimal(burnRatePercentPerHour) }
}

public struct TraySubscriptionUsageV1: Codable, Equatable, Sendable {
    public var planLabel: String?
    public var windows: [TrayQuotaWindowV1]
    public var manualResetsRemaining: Int64?
    public var manualResetCredits: [ManualResetCreditV1]?
}

public struct ManualResetCreditV1: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var title: String?
    public var expiresAt: String
}

public struct TrayMeteredUsageV1: Codable, Equatable, Sendable {
    public var todayCostUsd: String?
    public var rolling30DayCostUsd: String?
    public var dailyBudgetUsd: String?
    public var budgetConsumedPercent: String?
    public var totalTokens: UInt64
    public var costQuality: TrayCostQuality
    public var rhythmAdjustment: String?
    public var flatStatus: TrayUsageStatus?

    public var todayCost: Decimal? { decimal(todayCostUsd) }
    public var rolling30DayCost: Decimal? { decimal(rolling30DayCostUsd) }
    public var dailyBudget: Decimal? { decimal(dailyBudgetUsd) }
    public var consumedPercent: Decimal? { decimal(budgetConsumedPercent) }
}

private func decimal(_ value: String?) -> Decimal? {
    value.flatMap { Decimal(string: $0, locale: Locale(identifier: "en_US_POSIX")) }
}
