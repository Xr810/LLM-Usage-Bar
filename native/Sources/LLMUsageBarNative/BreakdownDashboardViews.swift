#if os(macOS)
import SwiftUI
import UsageCore

private enum ModelGrouping: String, CaseIterable, Identifiable {
    case plan
    case model
    var id: String { rawValue }
}

struct ModelBreakdownView: View {
    let dashboard: ModelUsageDashboardV1
    let isLoading: Bool
    let errorMessage: String?
    let rangeLabel: String
    @Binding var rangeSelection: UsageRangeSelectionV1

    @State private var grouping = ModelGrouping.plan

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                DashboardSummaryCard(
                    stats: summaryStats,
                    rangeLabel: rangeLabel,
                    countLabel: "\(dashboard.models.count) \(L10n.text(.models).lowercased())",
                    rangeSelection: $rangeSelection
                )
                if let errorMessage { DashboardMessageBanner(message: errorMessage) }
                ForEach(dashboard.warnings, id: \.self) { DashboardMessageBanner(message: $0) }
                if isLoading && dashboard.models.isEmpty {
                    DashboardLoadingState()
                } else if dashboard.models.isEmpty {
                    NativeCard {
                        DashboardEmptyState(title: L10n.text(.noData), systemImage: "cpu")
                            .frame(minHeight: 180)
                    }
                } else {
                    breakdownCard
                }
            }
            .padding(20)
        }
    }

    private var summaryStats: [DashboardStat] {
        [
            DashboardStat(id: "tokens", label: L10n.text(.tokens), value: NativeFormatting.count(dashboard.totalTokens), accessibilityValue: NativeFormatting.exactCount(dashboard.totalTokens)),
            DashboardStat(id: "requests", label: L10n.text(.requests), value: dashboard.totalEventCount.formatted(), accessibilityValue: nil),
            DashboardStat(id: "cost", label: "USD", value: NativeFormatting.money(dashboard.totalCost, zeroWhenEmpty: dashboard.totalEventCount == 0), accessibilityValue: nil),
        ]
    }

    private var breakdownCard: some View {
        NativeCard(padding: 0) {
            VStack(spacing: 0) {
                HStack {
                    Text(L10n.text(.modelBreakdown))
                        .font(.system(size: 12, weight: .semibold))
                    Spacer()
                    Picker(L10n.text(.modelBreakdown), selection: $grouping) {
                        Text(L10n.text(.byPlan)).tag(ModelGrouping.plan)
                        Text(L10n.text(.byModel)).tag(ModelGrouping.model)
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .frame(width: 170)
                }
                .padding(12)
                BreakdownColumnHeader(primary: grouping == .plan ? L10n.text(.plan) : L10n.text(.model))
                Divider()
                if grouping == .plan {
                    ForEach(dashboard.productGroups) { group in
                        planRow(group)
                        Divider().opacity(0.45)
                    }
                } else {
                    ForEach(dashboard.models) { model in
                        ShareBarRow(
                            title: model.model,
                            subtitle: model.providerIds.joined(separator: " · "),
                            share: usageShare(model.totalTokens, total: dashboard.totalTokens),
                            tokens: model.totalTokens,
                            requests: model.eventCount,
                            cost: model.totalCost,
                            costCanBeZero: model.eventCount == 0,
                            badge: model.costSourceCounts.unavailable > 0 ? L10n.text(.costUnavailable) : nil
                        )
                        Divider().opacity(0.45)
                    }
                }
            }
        }
    }

    private func planRow(_ group: ModelProductGroupV1) -> some View {
        ShareBarRow(
            title: productGroupName(group),
            subtitle: group.providerNames.joined(separator: " · "),
            share: usageShare(group.totalTokens, total: dashboard.totalTokens),
            tokens: group.totalTokens,
            requests: group.eventCount,
            cost: group.totalCost,
            costCanBeZero: group.eventCount == 0,
            badge: group.billingKind == .subscription ? L10n.text(.subscription) : L10n.text(.metered),
            expandable: true,
            leading: {
                NativeProviderIcon(
                    systemPresetKey: nil,
                    productGroupId: group.productGroupId,
                    name: productGroupName(group),
                    size: 23
                )
            },
            expanded: {
                VStack(spacing: 0) {
                    ForEach(group.models) { model in
                        NestedBreakdownRow(
                            title: model.model,
                            subtitle: group.providerIds.count > 1 ? model.providerName : nil,
                            share: usageShare(model.totalTokens, total: group.totalTokens),
                            tokens: model.totalTokens,
                            requests: model.eventCount,
                            cost: model.totalCost,
                            costCanBeZero: model.eventCount == 0
                        )
                    }
                }
            }
        )
    }

    private func productGroupName(_ group: ModelProductGroupV1) -> String {
        group.providerNames.first ?? group.productGroupId
    }
}

struct AgentBreakdownView: View {
    let dashboard: AgentUsageBreakdownV1
    let isLoading: Bool
    let errorMessage: String?
    let rangeLabel: String
    @Binding var rangeSelection: UsageRangeSelectionV1

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                DashboardSummaryCard(
                    stats: summaryStats,
                    rangeLabel: rangeLabel,
                    countLabel: "\(dashboard.agents.count) \(L10n.text(.agents).lowercased())",
                    rangeSelection: $rangeSelection
                )
                if let errorMessage { DashboardMessageBanner(message: errorMessage) }
                ForEach(dashboard.warnings, id: \.self) { DashboardMessageBanner(message: $0) }
                if isLoading && dashboard.agents.isEmpty {
                    DashboardLoadingState()
                } else if dashboard.agents.isEmpty {
                    NativeCard {
                        DashboardEmptyState(title: L10n.text(.noData), systemImage: "terminal")
                            .frame(minHeight: 180)
                    }
                } else {
                    agentCard
                }
            }
            .padding(20)
        }
    }

    private var summaryStats: [DashboardStat] {
        [
            DashboardStat(id: "tokens", label: L10n.text(.tokens), value: NativeFormatting.count(dashboard.totalTokens), accessibilityValue: NativeFormatting.exactCount(dashboard.totalTokens)),
            DashboardStat(id: "requests", label: L10n.text(.requests), value: dashboard.totalEventCount.formatted(), accessibilityValue: nil),
            DashboardStat(id: "cost", label: "USD", value: NativeFormatting.money(dashboard.totalCost, zeroWhenEmpty: dashboard.totalEventCount == 0), accessibilityValue: nil),
        ]
    }

    private var agentCard: some View {
        NativeCard(padding: 0) {
            VStack(spacing: 0) {
                Text(L10n.text(.agentBreakdown))
                    .font(.system(size: 12, weight: .semibold))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                BreakdownColumnHeader(primary: L10n.text(.agents))
                Divider()
                ForEach(dashboard.agents) { agent in
                    agentRow(agent)
                    Divider().opacity(0.45)
                }
            }
        }
    }

    private func agentRow(_ agent: AgentUsageRowV1) -> some View {
        ShareBarRow(
            title: agent.agentName ?? L10n.text(.unassigned),
            subtitle: "\(agent.providers.count) \(L10n.text(.accountCount))",
            share: usageShare(agent.totalTokens, total: dashboard.totalTokens),
            tokens: agent.totalTokens,
            requests: agent.eventCount,
            cost: agent.totalCost,
            costCanBeZero: agent.eventCount == 0,
            badge: agent.agentModuleId == nil ? L10n.text(.unassigned) : agent.archived ? L10n.text(.archived) : nil,
            expandable: true,
            leading: {
                Image(systemName: "terminal")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 23, height: 23)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
            },
            expanded: {
                VStack(alignment: .leading, spacing: 0) {
                    NativeSectionHeading(title: L10n.text(.accounts), count: agent.providers.count)
                        .padding(.horizontal, 11)
                        .padding(.top, 8)
                    ForEach(agent.providers) { provider in
                        NestedBreakdownRow(
                            title: provider.providerName,
                            subtitle: provider.billingKind == .subscription ? L10n.text(.subscription) : L10n.text(.metered),
                            share: usageShare(provider.totalTokens, total: agent.totalTokens),
                            tokens: provider.totalTokens,
                            requests: provider.eventCount,
                            cost: provider.totalCost,
                            costCanBeZero: provider.eventCount == 0
                        )
                    }
                    NativeSectionHeading(title: L10n.text(.models), count: agent.models.count)
                        .padding(.horizontal, 11)
                        .padding(.top, 8)
                    ForEach(agent.models) { model in
                        NestedBreakdownRow(
                            title: model.model,
                            subtitle: nil,
                            share: usageShare(model.totalTokens, total: agent.totalTokens),
                            tokens: model.totalTokens,
                            requests: model.eventCount,
                            cost: model.totalCost,
                            costCanBeZero: model.eventCount == 0
                        )
                    }
                }
                .padding(.bottom, 6)
            }
        )
    }
}

private struct BreakdownColumnHeader: View {
    let primary: String

    var body: some View {
        HStack(spacing: 9) {
            Text(primary.uppercased())
            Spacer()
            Text(L10n.text(.share).uppercased()).frame(width: 44, alignment: .trailing)
            Text(L10n.text(.tokens).uppercased()).frame(width: 62, alignment: .trailing)
            Text(L10n.text(.requests).uppercased()).frame(width: 52, alignment: .trailing)
            Text("USD").frame(width: 88, alignment: .trailing)
        }
        .font(.system(size: 9, weight: .medium))
        .foregroundStyle(.secondary)
        .padding(.horizontal, 11)
        .padding(.bottom, 7)
    }
}

private struct NestedBreakdownRow: View {
    let title: String
    let subtitle: String?
    let share: Double
    let tokens: UInt64
    let requests: UInt64
    let cost: Decimal?
    let costCanBeZero: Bool

    var body: some View {
        HStack(spacing: 9) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.system(size: 10, weight: .medium, design: .monospaced))
                    .lineLimit(1)
                if let subtitle {
                    Text(subtitle).font(.system(size: 9)).foregroundStyle(.secondary)
                }
            }
            Spacer()
            Text(share.formatted(.number.precision(.fractionLength(0))) + "%")
                .frame(width: 44, alignment: .trailing)
                .foregroundStyle(.secondary)
            Text(NativeFormatting.count(tokens)).frame(width: 62, alignment: .trailing)
            Text(requests.formatted()).frame(width: 52, alignment: .trailing).foregroundStyle(.secondary)
            Text(NativeFormatting.money(cost, zeroWhenEmpty: costCanBeZero)).frame(width: 88, alignment: .trailing)
        }
        .font(.system(size: 10).monospacedDigit())
        .padding(.horizontal, 11)
        .padding(.vertical, 7)
    }
}
#endif
