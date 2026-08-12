#if os(macOS)
import SwiftUI
import UsageCore

struct ProviderActivityDay: Identifiable, Equatable {
    let date: Date
    let eventCount: UInt64
    let totalTokens: UInt64
    let totalCost: Decimal?
    let level: Int

    var id: Date { date }
}

struct ProviderActivityHeatmap: View {
    @Environment(\.calendar) private var calendar
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    let buckets: [DashboardTrendBucketV1]
    let isLoading: Bool
    let errorMessage: String?

    @State private var hoveredDay: ProviderActivityDay?
    @FocusState private var focusedDayID: Date?

    private var days: [ProviderActivityDay] {
        buildActivityDays(buckets: buckets, calendar: calendar)
    }

    private var cells: [ProviderActivityDay?] {
        guard let first = days.first else { return [] }
        let leading = calendar.component(.weekday, from: first.date) - 1
        return Array(repeating: nil, count: leading) + days.map(Optional.some)
    }

    private var activeDays: Int { days.filter { $0.eventCount > 0 }.count }
    private var detailDay: ProviderActivityDay? {
        hoveredDay ?? days.first(where: { $0.id == focusedDayID })
    }

    var body: some View {
        NativeCard {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .top, spacing: 16) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(L10n.text(.dailyActivity))
                            .font(.system(size: 13, weight: .semibold))
                        Text(L10n.text(.activityRange))
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 3) {
                        Text("\(activeDays.formatted()) \(L10n.text(.activeDays))")
                            .font(.system(size: 11).monospacedDigit())
                            .foregroundStyle(.secondary)
                        detailText
                    }
                    .frame(minWidth: 210, alignment: .trailing)
                }

                if isLoading {
                    RoundedRectangle(cornerRadius: 8)
                        .fill(NativePalette.recessed(colorScheme))
                        .frame(height: 122)
                        .overlay { ProgressView().controlSize(.small) }
                } else if let errorMessage {
                    Label(errorMessage, systemImage: "chart.bar.xaxis")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, minHeight: 100)
                } else if days.isEmpty {
                    DashboardEmptyState(title: L10n.text(.noData), systemImage: "square.grid.3x3")
                        .frame(maxWidth: .infinity, minHeight: 100)
                } else {
                    heatmap
                }
            }
        }
    }

    private var detailText: some View {
        Group {
            if let detailDay {
                Text(
                    "\(detailDay.date.formatted(date: .abbreviated, time: .omitted)) · "
                        + "\(NativeFormatting.count(detailDay.totalTokens)) · "
                        + NativeFormatting.money(detailDay.totalCost, zeroWhenEmpty: detailDay.eventCount == 0)
                )
            } else {
                Text(L10n.text(.latestActivity))
                    .foregroundStyle(.tertiary)
            }
        }
        .font(.system(size: 10).monospacedDigit())
        .lineLimit(1)
    }

    private var heatmap: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 7) {
                monthLabels
                LazyHGrid(
                    rows: Array(repeating: GridItem(.fixed(14), spacing: 3), count: 7),
                    alignment: .top,
                    spacing: 3
                ) {
                    ForEach(Array(cells.enumerated()), id: \.offset) { index, day in
                        if let day {
                            activityCell(day, index: index)
                        } else {
                            Color.clear.frame(width: 14, height: 14)
                        }
                    }
                }
                .frame(height: 116)
                HStack(spacing: 5) {
                    Text("Low")
                    ForEach(0..<5, id: \.self) { level in
                        RoundedRectangle(cornerRadius: 2)
                            .fill(activityColor(level: level))
                            .frame(width: 10, height: 10)
                    }
                    Text("High")
                }
                .font(.system(size: 9))
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .trailing)
            }
            .padding(.bottom, 2)
        }
        .accessibilityLabel(L10n.text(.dailyActivity))
    }

    private var monthLabels: some View {
        HStack(spacing: 0) {
            ForEach(Array(monthMarkers.enumerated()), id: \.offset) { _, marker in
                Text(marker.label)
                    .font(.system(size: 9))
                    .foregroundStyle(.secondary)
                    .frame(width: CGFloat(max(1, marker.weekSpan)) * 17, alignment: .leading)
            }
        }
        .padding(.leading, CGFloat((calendar.component(.weekday, from: days[0].date) - 1) / 7) * 17)
    }

    private var monthMarkers: [(offset: Int, weekSpan: Int, label: String)] {
        guard !days.isEmpty else { return [] }
        var markers: [(Int, String)] = []
        var lastMonth = -1
        for (index, day) in days.enumerated() {
            let month = calendar.component(.month, from: day.date)
            if month != lastMonth {
                markers.append((index / 7, day.date.formatted(.dateTime.month(.abbreviated))))
                lastMonth = month
            }
        }
        return markers.enumerated().map { index, marker in
            let nextOffset = index + 1 < markers.count ? markers[index + 1].0 : Int(ceil(Double(cells.count) / 7.0))
            return (marker.0, max(1, nextOffset - marker.0), marker.1)
        }
    }

    private func activityCell(_ day: ProviderActivityDay, index: Int) -> some View {
        Button {
            focusedDayID = day.id
        } label: {
            RoundedRectangle(cornerRadius: 3)
                .fill(activityColor(level: day.level))
                .frame(width: 12, height: 12)
                .scaleEffect(hoveredDay?.id == day.id && !reduceMotion ? 1.18 : 1)
        }
        .buttonStyle(.plain)
        .focused($focusedDayID, equals: day.id)
        .onHover { hovering in hoveredDay = hovering ? day : nil }
        .onMoveCommand { direction in moveFocus(from: index, direction: direction) }
        .help(activityLabel(day))
        .accessibilityLabel(activityLabel(day))
    }

    private func activityColor(level: Int) -> Color {
        if level == 0 { return NativePalette.recessed(colorScheme) }
        return NativePalette.primary(colorScheme).opacity(0.20 + Double(level) * 0.17)
    }

    private func activityLabel(_ day: ProviderActivityDay) -> String {
        "\(day.date.formatted(date: .long, time: .omitted)): "
            + "\(NativeFormatting.exactCount(day.totalTokens)) \(L10n.text(.tokens)), "
            + NativeFormatting.money(day.totalCost, zeroWhenEmpty: day.eventCount == 0)
    }

    private func moveFocus(from cellIndex: Int, direction: MoveCommandDirection) {
        let delta: Int
        switch direction {
        case .left: delta = -7
        case .right: delta = 7
        case .up: delta = -1
        case .down: delta = 1
        default: return
        }
        var next = cellIndex + delta
        while cells.indices.contains(next) {
            if let target = cells[next] {
                focusedDayID = target.id
                return
            }
            next += delta.signum()
        }
    }
}

func buildActivityDays(
    buckets: [DashboardTrendBucketV1],
    calendar: Calendar = .current
) -> [ProviderActivityDay] {
    guard let first = buckets.min(by: { $0.startAt < $1.startAt }),
          let last = buckets.max(by: { $0.endAt < $1.endAt })
    else { return [] }

    var aggregate: [Date: (events: UInt64, tokens: UInt64, cost: Decimal?)] = [:]
    for bucket in buckets {
        let date = calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(bucket.startAt)))
        let current = aggregate[date] ?? (0, 0, nil)
        let cost = [current.cost, bucket.totalCost].compactMap { $0 }.reduce(0, +)
        aggregate[date] = (
            current.events + bucket.eventCount,
            current.tokens + bucket.totalTokens,
            cost == 0 && current.cost == nil && bucket.totalCost == nil ? nil : cost
        )
    }

    let start = calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(first.startAt)))
    let end = calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(max(first.startAt, last.endAt - 1))))
    var dates: [Date] = []
    var cursor = start
    while cursor <= end, dates.count < 366 {
        dates.append(cursor)
        cursor = calendar.date(byAdding: .day, value: 1, to: cursor) ?? cursor.addingTimeInterval(86_400)
    }
    let peak = aggregate.values.map { $0.tokens }.max() ?? 0
    return dates.map { date in
        let value = aggregate[date] ?? (0, 0, nil)
        let level: Int
        if value.tokens == 0 || peak == 0 {
            level = 0
        } else {
            level = min(4, max(1, Int(ceil(Double(value.tokens) / Double(peak) * 4))))
        }
        return ProviderActivityDay(
            date: date,
            eventCount: value.events,
            totalTokens: value.tokens,
            totalCost: value.cost,
            level: level
        )
    }
}
#endif
