#if os(macOS)
import SwiftUI
import UsageCore

struct DashboardEmptyState: View {
    let title: String
    let systemImage: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: systemImage)
                .font(.system(size: 25, weight: .light))
                .foregroundStyle(.tertiary)
            Text(title)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
    }
}

struct DashboardMessageBanner: View {
    let message: String
    var warning = true

    var body: some View {
        Label(message, systemImage: warning ? "exclamationmark.triangle.fill" : "info.circle")
            .font(.system(size: 11))
            .foregroundStyle(warning ? Color.orange : Color.secondary)
            .padding(.horizontal, 11)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
    }
}

struct DashboardLoadingState: View {
    var body: some View {
        VStack(spacing: 12) {
            ProgressView().controlSize(.small)
            Text(L10n.text(.loading))
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, minHeight: 180)
    }
}

struct DashboardStat: Identifiable {
    let id: String
    let label: String
    let value: String
    var accessibilityValue: String?
}

struct DashboardSummaryCard: View {
    let stats: [DashboardStat]
    let rangeLabel: String
    let countLabel: String?
    @Binding var rangeSelection: UsageRangeSelectionV1

    var body: some View {
        NativeCard {
            HStack(alignment: .bottom, spacing: 20) {
                ForEach(stats) { stat in
                    VStack(alignment: .leading, spacing: 3) {
                        Text(stat.label)
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                        Text(stat.value)
                            .font(.system(size: 17, weight: .semibold).monospacedDigit())
                            .lineLimit(1)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityValue(stat.accessibilityValue ?? stat.value)
                }
                Spacer(minLength: 8)
                VStack(alignment: .trailing, spacing: 2) {
                    Text(rangeLabel)
                        .font(.system(size: 11, weight: .medium))
                    if let countLabel {
                        Text(countLabel)
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                    }
                }
                UsageRangeControl(selection: $rangeSelection)
            }
        }
    }
}

struct ShareBarRow<Leading: View, Expanded: View>: View {
    @Environment(\.colorScheme) private var colorScheme

    let title: String
    let subtitle: String?
    let share: Double
    let tokens: UInt64
    let requests: UInt64
    let cost: Decimal?
    let costCanBeZero: Bool
    let badge: String?
    let leading: Leading
    let expanded: Expanded
    let expandable: Bool

    @State private var isExpanded = false

    init(
        title: String,
        subtitle: String? = nil,
        share: Double,
        tokens: UInt64,
        requests: UInt64,
        cost: Decimal?,
        costCanBeZero: Bool = false,
        badge: String? = nil,
        expandable: Bool = false,
        @ViewBuilder leading: () -> Leading,
        @ViewBuilder expanded: () -> Expanded
    ) {
        self.title = title
        self.subtitle = subtitle
        self.share = share
        self.tokens = tokens
        self.requests = requests
        self.cost = cost
        self.costCanBeZero = costCanBeZero
        self.badge = badge
        self.expandable = expandable
        self.leading = leading()
        self.expanded = expanded()
    }

    var body: some View {
        VStack(spacing: 0) {
            Button {
                if expandable { isExpanded.toggle() }
            } label: {
                rowContent
            }
            .buttonStyle(.plain)
            .disabled(!expandable)
            .accessibilityValue(expandable ? (isExpanded ? "Expanded" : "Collapsed") : "")
            if expandable && isExpanded {
                expanded
                    .padding(.leading, 24)
                    .background(NativePalette.recessed(colorScheme))
            }
        }
    }

    private var rowContent: some View {
        ZStack(alignment: .leading) {
            GeometryReader { proxy in
                NativePalette.primary(colorScheme)
                    .opacity(0.09)
                    .frame(width: proxy.size.width * max(0, min(share, 100)) / 100)
            }
            HStack(spacing: 9) {
                if expandable {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                        .frame(width: 10)
                }
                leading
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(title)
                            .font(.system(size: 12, weight: .semibold))
                            .lineLimit(1)
                        if let badge { NativeBadge(text: badge) }
                    }
                    if let subtitle {
                        Text(subtitle)
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer()
                Text(share.formatted(.number.precision(.fractionLength(0))) + "%")
                    .frame(width: 44, alignment: .trailing)
                    .foregroundStyle(.secondary)
                Text(NativeFormatting.count(tokens))
                    .frame(width: 62, alignment: .trailing)
                    .fontWeight(.semibold)
                Text(NativeFormatting.exactCount(requests))
                    .frame(width: 52, alignment: .trailing)
                    .foregroundStyle(.secondary)
                Text(NativeFormatting.money(cost, zeroWhenEmpty: costCanBeZero))
                    .frame(width: 88, alignment: .trailing)
                    .foregroundStyle(cost == nil && !costCanBeZero ? .secondary : .primary)
            }
            .font(.system(size: 11).monospacedDigit())
            .padding(.horizontal, 11)
            .padding(.vertical, 9)
        }
        .contentShape(Rectangle())
    }
}

extension ShareBarRow where Leading == EmptyView, Expanded == EmptyView {
    init(
        title: String,
        subtitle: String? = nil,
        share: Double,
        tokens: UInt64,
        requests: UInt64,
        cost: Decimal?,
        costCanBeZero: Bool = false,
        badge: String? = nil
    ) {
        self.init(
            title: title,
            subtitle: subtitle,
            share: share,
            tokens: tokens,
            requests: requests,
            cost: cost,
            costCanBeZero: costCanBeZero,
            badge: badge,
            leading: { EmptyView() },
            expanded: { EmptyView() }
        )
    }
}

func usageShare(_ value: UInt64, total: UInt64) -> Double {
    guard total > 0 else { return 0 }
    return Double(value) / Double(total) * 100
}
#endif
