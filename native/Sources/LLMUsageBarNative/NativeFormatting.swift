#if os(macOS)
import Foundation
import SwiftUI
import UsageCore

@MainActor
enum NativeFormatting {
    static let currency: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.numberStyle = .currency
        formatter.currencyCode = "USD"
        formatter.maximumFractionDigits = 2
        return formatter
    }()

    static let relativeDate: RelativeDateTimeFormatter = {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter
    }()

    static func money(_ value: Decimal?) -> String {
        guard let value else { return L10n.text(.unavailable) }
        return currency.string(from: value as NSDecimalNumber) ?? "$\(value)"
    }

    static func money(_ value: Decimal?, zeroWhenEmpty isEmpty: Bool) -> String {
        if value == nil, isEmpty { return "$0" }
        return money(value)
    }

    static func percent(_ value: Decimal?) -> String {
        guard let value else { return L10n.text(.unavailable) }
        return "\(NSDecimalNumber(decimal: value).rounding(accordingToBehavior: nil))%"
    }

    static func reset(_ rfc3339: String?) -> String? {
        guard let rfc3339, let date = ISO8601DateFormatter().date(from: rfc3339) else { return nil }
        return relativeDate.localizedString(for: date, relativeTo: .now)
    }

    static func count(_ value: UInt64) -> String {
        value.formatted(.number.notation(.compactName))
    }

    static func exactCount(_ value: UInt64) -> String {
        value.formatted(.number)
    }

    static func date(_ timestamp: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(timestamp)).formatted(
            date: .abbreviated,
            time: .shortened
        )
    }

    static func relative(_ timestamp: Int64?) -> String {
        guard let timestamp else { return L10n.text(.unavailable) }
        return relativeDate.localizedString(
            for: Date(timeIntervalSince1970: TimeInterval(timestamp)),
            relativeTo: .now
        )
    }

    static func sourceLabels(_ sources: [TokenSourceV1]?) -> String? {
        guard let sources, !sources.isEmpty else { return nil }
        return sources.map {
            switch $0 {
            case .proxy: L10n.text(.sourceProxy)
            case .sessionLog: L10n.text(.sourceSession)
            }
        }
        .joined(separator: " + ")
    }
}

extension TrayUsageStatus {
    var color: Color {
        switch self {
        case .green: .green
        case .yellow: .orange
        case .red: .red
        case .unknown: .secondary
        }
    }
}

struct StatusLight: View {
    @Environment(\.colorScheme) private var colorScheme
    let status: TrayUsageStatus
    var size: CGFloat = 10

    var body: some View {
        Circle()
            .fill(NativePalette.status(status, scheme: colorScheme))
            .frame(width: size, height: size)
            .overlay(Circle().stroke(.primary.opacity(0.18), lineWidth: 0.5))
            .accessibilityHidden(true)
    }
}
#endif
