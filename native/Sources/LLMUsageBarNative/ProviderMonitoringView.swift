#if os(macOS)
import Charts
import SwiftUI
import UsageCore

struct ProviderMonitoringView: View {
    @ObservedObject var model: UsageAppModel
    let range: DashboardRangeV1
    let rangeLabel: String
    @Binding var rangeSelection: UsageRangeSelectionV1

    @State private var selectedProvider: ProviderDashboardRowV1?

    private var subscriptions: [ProviderDashboardRowV1] {
        model.providerDashboard.providers.filter { $0.provider.billingKind == .subscription }
    }

    private var metered: [ProviderDashboardRowV1] {
        model.providerDashboard.providers.filter { $0.provider.billingKind == .metered }
    }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 16) {
                if let error = model.providerErrorMessage {
                    DashboardMessageBanner(message: error)
                }
                ForEach(model.providerDashboard.warnings, id: \.self) { warning in
                    DashboardMessageBanner(message: warning)
                }

                if model.isProviderLoading && model.providerDashboard.providers.isEmpty {
                    DashboardLoadingState()
                } else if model.providerDashboard.providers.isEmpty {
                    NativeCard {
                        DashboardEmptyState(title: L10n.text(.noProviders), systemImage: "server.rack")
                            .frame(minHeight: 150)
                    }
                } else {
                    ProviderActivityHeatmap(
                        buckets: model.providerActivity,
                        isLoading: model.isActivityLoading,
                        errorMessage: model.activityErrorMessage
                    )
                    ProviderUsageTrendCard(
                        dashboard: model.providerDashboard,
                        rangeLabel: rangeLabel,
                        rangeSelection: $rangeSelection
                    )
                    subscriptionSection
                    meteredSection
                }
            }
            .padding(20)
        }
        .sheet(item: $selectedProvider) { provider in
            ProviderUsageEventsSheet(model: model, provider: provider, range: range)
                .frame(minWidth: 760, minHeight: 500)
        }
    }

    private var subscriptionSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                NativeSectionHeading(title: L10n.text(.remainingQuota), count: subscriptions.count)
                Spacer()
                Text("\(NativeFormatting.count(totalTokens)) \(L10n.text(.tokens)) · \(totalCalls.formatted()) \(L10n.text(.calls))")
                    .font(.system(size: 10).monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            if subscriptions.isEmpty {
                NativeCard {
                    Text(L10n.text(.noSubscriptionProviders))
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, minHeight: 50)
                }
            } else {
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 390, maximum: 620), spacing: 14, alignment: .top)],
                    alignment: .leading,
                    spacing: 14
                ) {
                    ForEach(subscriptions) { provider in
                        SubscriptionProviderCard(provider: provider)
                    }
                }
            }
        }
    }

    private var meteredSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            NativeSectionHeading(title: L10n.text(.meteredAccounts), count: metered.count)
            MeteredOverviewCard(providers: metered)
            if metered.isEmpty {
                NativeCard {
                    Text(L10n.text(.noMeteredProviders))
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, minHeight: 50)
                }
            } else {
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 390, maximum: 620), spacing: 14, alignment: .top)],
                    alignment: .leading,
                    spacing: 14
                ) {
                    ForEach(metered) { provider in
                        MeteredProviderCard(
                            provider: provider,
                            recentEvents: model.recentEventsByProvider[provider.id],
                            onViewAll: { selectedProvider = provider }
                        )
                    }
                }
            }
        }
    }

    private var totalTokens: UInt64 {
        model.providerDashboard.providers.reduce(0) { $0 + $1.totalTokens }
    }

    private var totalCalls: UInt64 {
        model.providerDashboard.providers.reduce(0) { $0 + $1.eventCount }
    }
}

private struct ProviderUsageTrendCard: View {
    let dashboard: ProviderMonitoringDashboardV1
    let rangeLabel: String
    @Binding var rangeSelection: UsageRangeSelectionV1

    private var totalTokens: UInt64 {
        dashboard.providers.reduce(0) { $0 + $1.totalTokens }
    }

    private var totalCost: Decimal? {
        let values = dashboard.providers.compactMap(\.totalCost)
        guard !values.isEmpty else { return nil }
        return values.reduce(0, +)
    }

    private var peakTokens: UInt64 {
        dashboard.trendBuckets.map(\.totalTokens).max() ?? 0
    }

    var body: some View {
        NativeCard {
            VStack(alignment: .leading, spacing: 13) {
                HStack(alignment: .top, spacing: 16) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(L10n.text(.usageTrend))
                            .font(.system(size: 13, weight: .semibold))
                        Text("\(dashboard.trendGranularity == .hour ? L10n.text(.hourly) : L10n.text(.daily)) · \(rangeLabel)")
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 3) {
                        HStack(spacing: 6) {
                            Text(NativeFormatting.money(totalCost))
                                .font(.system(size: 17, weight: .semibold).monospacedDigit())
                            if dashboard.providers.contains(where: { $0.eventCount > 0 && $0.totalCost == nil }) {
                                NativeBadge(text: L10n.text(.costUnavailable))
                            }
                        }
                        Text("\(L10n.text(.peak)) \(NativeFormatting.count(peakTokens)) · \(NativeFormatting.count(totalTokens)) \(L10n.text(.tokens))")
                            .font(.system(size: 10).monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }
                NativeRecessedSurface {
                    HStack {
                        Text(L10n.text(.timeRange))
                            .font(.system(size: 11, weight: .medium))
                        Spacer()
                        UsageRangeControl(selection: $rangeSelection)
                    }
                }
                if dashboard.trendBuckets.contains(where: { $0.totalTokens > 0 }) {
                    Chart(dashboard.trendBuckets) { bucket in
                        AreaMark(
                            x: .value("Date", Date(timeIntervalSince1970: TimeInterval(bucket.startAt))),
                            y: .value("Tokens", bucket.totalTokens)
                        )
                        .interpolationMethod(.catmullRom)
                        .foregroundStyle(.linearGradient(
                            colors: [NativePalette.primary(.dark).opacity(0.28), NativePalette.primary(.dark).opacity(0.02)],
                            startPoint: .top,
                            endPoint: .bottom
                        ))
                        LineMark(
                            x: .value("Date", Date(timeIntervalSince1970: TimeInterval(bucket.startAt))),
                            y: .value("Tokens", bucket.totalTokens)
                        )
                        .interpolationMethod(.catmullRom)
                        .foregroundStyle(NativePalette.primary(.dark))
                        .lineStyle(StrokeStyle(lineWidth: 2))
                    }
                    .chartYAxis(.hidden)
                    .chartXAxis {
                        AxisMarks(values: .automatic(desiredCount: 6)) { _ in
                            AxisGridLine().foregroundStyle(.secondary.opacity(0.12))
                            AxisValueLabel(format: .dateTime.month().day())
                                .font(.system(size: 9))
                        }
                    }
                    .frame(height: 220)
                    .accessibilityLabel(L10n.text(.usageTrend))
                } else {
                    DashboardEmptyState(title: L10n.text(.noData), systemImage: "chart.xyaxis.line")
                        .frame(minHeight: 150)
                }
            }
        }
    }
}

private struct SubscriptionProviderCard: View {
    let provider: ProviderDashboardRowV1

    private var freshness: Int64? {
        provider.quotaFetchState?.lastSuccessAt ?? provider.quota?.sourceObservedAt ?? provider.quota?.fetchedAt
    }

    var body: some View {
        NativeCard(padding: 0) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    NativeProviderIcon(
                        systemPresetKey: provider.provider.systemPresetKey,
                        productGroupId: provider.provider.productGroupId,
                        name: provider.provider.name
                    )
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(displayName)
                                .font(.system(size: 13, weight: .semibold))
                                .lineLimit(1)
                            NativeBadge(text: L10n.text(.subscription))
                        }
                        Text("\(L10n.text(.lastUpdated)) \(NativeFormatting.relative(freshness))")
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if provider.quotaFetchState?.stale == true {
                        NativeBadge(text: L10n.text(.stale), status: .yellow)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 14)

                VStack(spacing: 9) {
                    QuotaMeterRow(
                        title: L10n.text(.fiveHourWindow),
                        remaining: provider.quota?.fiveHourRemainingPercent,
                        reset: provider.quota?.fiveHourResetsAt,
                        pace: provider.quota?.fiveHourPace
                    )
                    QuotaMeterRow(
                        title: L10n.text(.weeklyAllowance),
                        remaining: provider.quota?.sevenDayRemainingPercent,
                        reset: provider.quota?.sevenDayResetsAt,
                        pace: provider.quota?.sevenDayPace
                    )
                }
                .padding(.horizontal, 16)
                .padding(.top, 13)

                if let resets = provider.quota?.manualResetsRemaining {
                    HStack(spacing: 5) {
                        Image(systemName: "arrow.counterclockwise.circle")
                        Text("\(L10n.text(.resetCredits)): \(resets)")
                    }
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 16)
                    .padding(.top, 8)
                }

                NativeRecessedSurface {
                    HStack(spacing: 20) {
                        compactMetric(L10n.text(.tokens), NativeFormatting.count(provider.totalTokens))
                        compactMetric(L10n.text(.calls), provider.eventCount.formatted())
                        Spacer()
                    }
                }
                .padding(16)
            }
        }
    }

    private var displayName: String {
        guard let plan = provider.quota?.planType, !plan.isEmpty else { return provider.provider.name }
        return provider.provider.name.localizedCaseInsensitiveContains(plan)
            ? provider.provider.name
            : "\(provider.provider.name) · \(plan.capitalized)"
    }

    private func compactMetric(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.system(size: 9)).foregroundStyle(.secondary)
            Text(value).font(.system(size: 12, weight: .semibold).monospacedDigit())
        }
    }
}

private struct QuotaMeterRow: View {
    @Environment(\.colorScheme) private var colorScheme
    let title: String
    let remaining: Decimal?
    let reset: String?
    let pace: QuotaWindowPaceV1?

    private var status: TrayUsageStatus { pace?.status ?? .unknown }

    var body: some View {
        NativeRecessedSurface {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(title).font(.system(size: 11, weight: .medium))
                    Spacer()
                    Text(remaining.map { "\(NativeFormatting.percent($0)) \(L10n.text(.remaining))" } ?? L10n.text(.unavailable))
                        .font(.system(size: 11, weight: .semibold).monospacedDigit())
                        .foregroundStyle(NativePalette.status(status, scheme: colorScheme))
                }
                if let remaining {
                    ProgressView(value: NSDecimalNumber(decimal: remaining).doubleValue, total: 100)
                        .progressViewStyle(.linear)
                        .tint(NativePalette.status(status, scheme: colorScheme))
                        .frame(height: 5)
                } else {
                    Capsule().fill(NativePalette.recessed(colorScheme)).frame(height: 5)
                }
                HStack(spacing: 7) {
                    Text(NativeFormatting.reset(reset) ?? (remaining == nil ? L10n.text(.unavailable) : L10n.text(.resetTimeUnknown)))
                    if let burn = pace?.burnRatePerHour {
                        Text("· \(NativeFormatting.percent(burn))/h")
                    }
                    Spacer()
                }
                .font(.system(size: 9).monospacedDigit())
                .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

private struct MeteredOverviewCard: View {
    let providers: [ProviderDashboardRowV1]

    private var tokens: UInt64 { providers.reduce(0) { $0 + $1.totalTokens } }
    private var requests: UInt64 { providers.reduce(0) { $0 + $1.eventCount } }
    private var cost: Decimal? {
        let values = providers.compactMap(\.totalCost)
        return values.isEmpty ? nil : values.reduce(0, +)
    }

    var body: some View {
        NativeCard {
            HStack(spacing: 24) {
                Text(L10n.text(.meteredOverview))
                    .font(.system(size: 12, weight: .semibold))
                Spacer()
                overviewMetric(L10n.text(.tokens), NativeFormatting.count(tokens))
                overviewMetric(L10n.text(.requests), requests.formatted())
                overviewMetric("USD", NativeFormatting.money(cost, zeroWhenEmpty: requests == 0))
            }
        }
    }

    private func overviewMetric(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.system(size: 9)).foregroundStyle(.secondary)
            Text(value).font(.system(size: 13, weight: .semibold).monospacedDigit())
        }
    }
}

private struct MeteredProviderCard: View {
    let provider: ProviderDashboardRowV1
    let recentEvents: UsageEventPageV1?
    let onViewAll: () -> Void

    var body: some View {
        NativeCard(padding: 0) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    NativeProviderIcon(
                        systemPresetKey: provider.provider.systemPresetKey,
                        productGroupId: provider.provider.productGroupId,
                        name: provider.provider.name,
                        size: 32
                    )
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(provider.provider.name)
                                .font(.system(size: 13, weight: .semibold))
                                .lineLimit(1)
                            NativeBadge(text: L10n.text(.metered))
                        }
                        if let sources = NativeFormatting.sourceLabels(provider.tokenSources) {
                            Text(sources).font(.system(size: 10)).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    if provider.costSourceCounts.estimated > 0 { NativeBadge(text: "Estimated") }
                    if provider.costSourceCounts.unavailable > 0 { NativeBadge(text: L10n.text(.costUnavailable)) }
                }
                .padding(.horizontal, 16)
                .padding(.top, 14)

                NativeRecessedSurface {
                    HStack(spacing: 20) {
                        cardMetric(L10n.text(.tokens), NativeFormatting.count(provider.totalTokens))
                        cardMetric(L10n.text(.requests), provider.eventCount.formatted())
                        cardMetric("USD", NativeFormatting.money(provider.totalCost, zeroWhenEmpty: provider.eventCount == 0))
                        Spacer()
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 13)

                if let recentEvents, !recentEvents.items.isEmpty {
                    Divider().padding(.top, 13)
                    HStack {
                        NativeSectionHeading(title: L10n.text(.recentRequests))
                        Spacer()
                        Button(L10n.text(.viewAll), action: onViewAll)
                            .buttonStyle(.link)
                            .font(.system(size: 10))
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 10)
                    VStack(spacing: 6) {
                        ForEach(recentEvents.items.prefix(5)) { event in
                            HStack(spacing: 8) {
                                Text(event.model)
                                    .font(.system(size: 10, design: .monospaced))
                                    .lineLimit(1)
                                Spacer()
                                Text(NativeFormatting.money(event.totalCost))
                                    .font(.system(size: 10).monospacedDigit())
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 7)
                    .padding(.bottom, 14)
                } else {
                    Color.clear.frame(height: 14)
                }
            }
        }
    }

    private func cardMetric(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.system(size: 9)).foregroundStyle(.secondary)
            Text(value).font(.system(size: 12, weight: .semibold).monospacedDigit())
        }
    }
}

private struct ProviderUsageEventsSheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: UsageAppModel
    let provider: ProviderDashboardRowV1
    let range: DashboardRangeV1

    @State private var page: UInt64 = 1
    private let pageSize: UInt64 = 50

    var body: some View {
        ZStack {
            NativeWindowBackground()
            VStack(alignment: .leading, spacing: 14) {
                HStack {
                    NativeProviderIcon(
                        systemPresetKey: provider.provider.systemPresetKey,
                        productGroupId: provider.provider.productGroupId,
                        name: provider.provider.name
                    )
                    VStack(alignment: .leading, spacing: 2) {
                        Text(provider.provider.name).font(.headline)
                        Text(L10n.text(.recentRequests)).font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button(L10n.text(.cancel)) { dismiss() }
                }
                if model.eventsProviderId == provider.id, !model.usageEvents.items.isEmpty {
                    Table(model.usageEvents.items) {
                        TableColumn(L10n.text(.model)) { event in Text(event.model).lineLimit(1) }
                        TableColumn(L10n.text(.lastUpdated)) { event in Text(NativeFormatting.date(event.occurredAt)) }
                        TableColumn(L10n.text(.tokens)) { event in Text(NativeFormatting.count(event.totalTokens)).monospacedDigit() }
                        TableColumn(L10n.text(.totalCost)) { event in Text(NativeFormatting.money(event.totalCost)).monospacedDigit() }
                    }
                    HStack {
                        Text("\(model.usageEvents.total.formatted()) \(L10n.text(.requests))")
                            .foregroundStyle(.secondary)
                        Spacer()
                        Button(L10n.text(.previous)) { page = max(1, page - 1) }
                            .disabled(page <= 1)
                        Text(page.formatted()).monospacedDigit()
                        Button(L10n.text(.next)) { page += 1 }
                            .disabled(page * pageSize >= model.usageEvents.total)
                    }
                    .font(.system(size: 11))
                } else {
                    DashboardLoadingState()
                }
            }
            .padding(18)
        }
        .task(id: "\(provider.id):\(page)") {
            await model.loadUsageEvents(
                providerId: provider.id,
                range: range,
                page: page,
                pageSize: pageSize
            )
        }
    }
}
#endif
