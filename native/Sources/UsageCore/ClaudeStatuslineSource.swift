import Foundation

/// Read-only adapter for the cache maintained by the existing Rust status-line bridge.
/// It deliberately reads neither Claude credentials nor the application database.
public struct ClaudeStatuslineSource: Sendable {
    public enum SourceError: Error, Equatable {
        case unsupportedSchema(Int)
        case staleCache
        case noCurrentWindows
    }

    private struct Cache: Decodable {
        let schemaVersion: Int
        let updatedAt: Int64
        let sessions: [Session]
    }

    private struct Session: Decodable {
        let rateLimits: RateLimits?
    }

    private struct RateLimits: Decodable {
        let fiveHour: CachedWindow?
        let sevenDay: CachedWindow?
    }

    private struct CachedWindow: Decodable {
        let usedPercentage: Double
        let resetsAt: Int64
        let observedAt: Int64
    }

    public static let supportedSchemaVersion = 5
    public static let maximumAge: TimeInterval = 15 * 60

    public let fileURL: URL

    public init(fileURL: URL = Self.defaultFileURL()) {
        self.fileURL = fileURL
    }

    public static func defaultFileURL(homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser) -> URL {
        homeDirectory
            .appendingPathComponent(".llm-usage-bar", isDirectory: true)
            .appendingPathComponent("runtime", isDirectory: true)
            .appendingPathComponent("claude-statusline-quota.json")
    }

    public func load(now: Date = .now) throws -> ProviderUsage {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let cache = try decoder.decode(Cache.self, from: Data(contentsOf: fileURL))

        guard cache.schemaVersion == Self.supportedSchemaVersion else {
            throw SourceError.unsupportedSchema(cache.schemaVersion)
        }
        guard now.timeIntervalSince1970 - Double(cache.updatedAt) <= Self.maximumAge else {
            throw SourceError.staleCache
        }

        let fiveHour = newestWindow(in: cache.sessions, keyPath: \RateLimits.fiveHour)
            .flatMap { makeWindow($0, id: "claude-five-hour", title: "5 hour window", now: now) }
        let sevenDay = newestWindow(in: cache.sessions, keyPath: \RateLimits.sevenDay)
            .flatMap { makeWindow($0, id: "claude-seven-day", title: "Weekly window", now: now) }
        let windows = [fiveHour, sevenDay].compactMap { $0 }
        guard !windows.isEmpty else { throw SourceError.noCurrentWindows }

        return ProviderUsage(id: "claude", name: "Claude", windows: windows)
    }

    private func newestWindow(
        in sessions: [Session],
        keyPath: KeyPath<RateLimits, CachedWindow?>
    ) -> CachedWindow? {
        sessions
            .compactMap(\.rateLimits)
            .compactMap { $0[keyPath: keyPath] }
            .max { $0.observedAt < $1.observedAt }
    }

    private func makeWindow(_ window: CachedWindow, id: String, title: String, now: Date) -> QuotaWindow? {
        guard window.resetsAt > Int64(now.timeIntervalSince1970) else { return nil }
        return QuotaWindow(
            id: id,
            title: title,
            utilizationPercent: window.usedPercentage,
            resetsAt: Date(timeIntervalSince1970: TimeInterval(window.resetsAt)),
            health: health(for: window.usedPercentage)
        )
    }

    private func health(for usedPercentage: Double) -> UsageHealth {
        if usedPercentage >= 90 { return .critical }
        if usedPercentage >= 75 { return .warning }
        return .healthy
    }
}
