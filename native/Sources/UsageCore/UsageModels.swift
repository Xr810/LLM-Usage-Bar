import Foundation

public enum UsageHealth: String, Codable, Sendable {
    case healthy
    case warning
    case critical
    case unknown

    public var symbolName: String {
        switch self {
        case .healthy: "checkmark.circle.fill"
        case .warning: "exclamationmark.triangle.fill"
        case .critical: "exclamationmark.octagon.fill"
        case .unknown: "questionmark.circle.fill"
        }
    }
}

public struct QuotaWindow: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let title: String
    public let utilizationPercent: Double?
    public let resetsAt: Date?
    public let health: UsageHealth

    public init(
        id: String,
        title: String,
        utilizationPercent: Double?,
        resetsAt: Date?,
        health: UsageHealth
    ) {
        self.id = id
        self.title = title
        self.utilizationPercent = utilizationPercent
        self.resetsAt = resetsAt
        self.health = health
    }

    public var remainingPercent: Double? {
        utilizationPercent.map { min(100, max(0, 100 - $0)) }
    }
}

public struct ProviderUsage: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let name: String
    public let windows: [QuotaWindow]

    public init(id: String, name: String, windows: [QuotaWindow]) {
        self.id = id
        self.name = name
        self.windows = windows
    }

    public var health: UsageHealth {
        if windows.contains(where: { $0.health == .critical }) { return .critical }
        if windows.contains(where: { $0.health == .warning }) { return .warning }
        if windows.contains(where: { $0.health == .healthy }) { return .healthy }
        return .unknown
    }
}

public struct UsageSnapshot: Codable, Equatable, Sendable {
    public let generatedAt: Date
    public let providers: [ProviderUsage]

    public init(generatedAt: Date, providers: [ProviderUsage]) {
        self.generatedAt = generatedAt
        self.providers = providers
    }

    public var health: UsageHealth {
        if providers.contains(where: { $0.health == .critical }) { return .critical }
        if providers.contains(where: { $0.health == .warning }) { return .warning }
        if providers.contains(where: { $0.health == .healthy }) { return .healthy }
        return .unknown
    }
}
