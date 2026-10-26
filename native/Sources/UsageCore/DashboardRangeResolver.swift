import Foundation

public enum UsageRangePresetV1: String, CaseIterable, Codable, Sendable {
    case today
    case sevenDays = "7d"
    case thirtyDays = "30d"
    case oneYear = "1y"
    case custom
}

public struct UsageRangeSelectionV1: Equatable, Sendable {
    public var preset: UsageRangePresetV1
    public var customStartAt: Int64?
    public var customEndAt: Int64?
    public var liveEndTime: Bool

    public init(
        preset: UsageRangePresetV1 = .thirtyDays,
        customStartAt: Int64? = nil,
        customEndAt: Int64? = nil,
        liveEndTime: Bool = false
    ) {
        self.preset = preset
        self.customStartAt = customStartAt
        self.customEndAt = customEndAt
        self.liveEndTime = liveEndTime
    }
}

public enum DashboardRangeResolver {
    private static let daySeconds: Int64 = 24 * 60 * 60

    public static func resolve(
        _ selection: UsageRangeSelectionV1,
        now: Date = .now,
        calendar: Calendar = .current
    ) -> DashboardRangeV1 {
        let liveEnd = Int64(now.timeIntervalSince1970) + 1
        let startOfToday = Int64(calendar.startOfDay(for: now).timeIntervalSince1970)

        switch selection.preset {
        case .today:
            return DashboardRangeV1(startAt: startOfToday, endAt: liveEnd)
        case .sevenDays:
            return calendarRange(days: 7, now: now, endAt: liveEnd, calendar: calendar)
        case .thirtyDays:
            return calendarRange(days: 30, now: now, endAt: liveEnd, calendar: calendar)
        case .oneYear:
            return calendarRange(days: 365, now: now, endAt: liveEnd, calendar: calendar)
        case .custom:
            let start = selection.customStartAt ?? liveEnd - daySeconds
            let requestedEnd = selection.liveEndTime
                ? liveEnd
                : selection.customEndAt ?? liveEnd
            let lower = min(start, requestedEnd)
            let upper = max(start, requestedEnd)
            return DashboardRangeV1(startAt: lower, endAt: upper == lower ? lower + 1 : upper)
        }
    }

    public static func providerActivity(
        now: Date = .now,
        calendar: Calendar = .current
    ) -> DashboardRangeV1 {
        let today = calendar.startOfDay(for: now)
        let start = calendar.date(byAdding: .day, value: -364, to: today) ?? today
        let end = calendar.date(byAdding: .day, value: 1, to: today)
            ?? today.addingTimeInterval(TimeInterval(daySeconds))
        return DashboardRangeV1(
            startAt: Int64(start.timeIntervalSince1970),
            endAt: Int64(end.timeIntervalSince1970)
        )
    }

    private static func calendarRange(
        days: Int,
        now: Date,
        endAt: Int64,
        calendar: Calendar
    ) -> DashboardRangeV1 {
        let today = calendar.startOfDay(for: now)
        let start = calendar.date(byAdding: .day, value: -(days - 1), to: today) ?? today
        return DashboardRangeV1(startAt: Int64(start.timeIntervalSince1970), endAt: endAt)
    }
}
