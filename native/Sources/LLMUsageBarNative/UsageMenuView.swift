#if os(macOS)
import AppKit
import SwiftUI
import UsageCore

struct UsageMenuView: View {
    @Environment(\.openWindow) private var openWindow
    @ObservedObject var model: UsageAppModel

    private var subscriptions: [TrayProviderUsageV1] {
        model.snapshot.providers.filter { $0.billingKind == .subscription && $0.subscription != nil }
    }

    private var metered: [TrayProviderUsageV1] {
        model.snapshot.providers.filter { $0.billingKind == .metered && $0.metered != nil }
    }

    var body: some View {
        ZStack {
            NativeWindowBackground()
            VStack(spacing: 0) {
                header
                Divider().opacity(0.55)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 10) {
                        if model.snapshot.stale || model.errorMessage != nil { stateBanner }
                        subscriptionSection
                        apiSpendingSection
                        if subscriptions.isEmpty && metered.isEmpty { emptyState }
                    }
                    .padding(12)
                }
                Divider().opacity(0.55)
                footer
            }
        }
        .frame(width: 380, height: 520)
        .controlSize(.small)
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(L10n.text(.providerMonitoring))
                        .font(.system(size: 13, weight: .semibold))
                    #if NATIVE_PREVIEW
                    if model.isUsingPreviewData { NativeBadge(text: L10n.text(.sampleData)) }
                    #endif
                }
                HStack(spacing: 4) {
                    Text("\(L10n.text(.lastUpdated)) \(NativeFormatting.relative(model.snapshot.lastSuccessAt))")
                    if model.snapshot.stale { Text("· \(L10n.text(.stale))").fontWeight(.medium) }
                }
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
            }
            Spacer()
            if model.isRefreshing || model.snapshot.refreshInProgress {
                ProgressView().controlSize(.mini)
            }
            NativeStatusBadge(status: model.snapshot.status)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 11)
        .background(.ultraThinMaterial)
        .accessibilityElement(children: .contain)
    }

    private var stateBanner: some View {
        DashboardMessageBanner(
            message: model.errorMessage ?? L10n.text(.stale),
            warning: model.errorMessage != nil
        )
    }

    private var subscriptionSection: some View {
        VStack(alignment: .leading, spacing: 7) {
            NativeSectionHeading(title: L10n.text(.remainingQuota), count: subscriptions.count)
                .padding(.horizontal, 2)
            ForEach(subscriptions) { provider in
                TraySubscriptionCard(provider: provider)
            }
        }
    }

    private var apiSpendingSection: some View {
        VStack(alignment: .leading, spacing: 7) {
            NativeSectionHeading(title: L10n.text(.meteredAccounts), count: metered.count)
                .padding(.horizontal, 2)
            TrayBudgetCard(budget: model.snapshot.apiBudget)
            ForEach(metered) { provider in
                TrayMeteredCard(provider: provider)
            }
        }
    }

    private var emptyState: some View {
        DashboardEmptyState(title: L10n.text(.noProviders), systemImage: "chart.bar.xaxis")
            .frame(minHeight: 120)
    }

    private var footer: some View {
        HStack(spacing: 7) {
            Button {
                AppLifecycle.shared.enterForegroundMode()
                openWindow(id: "main")
            } label: {
                Label(L10n.text(.details), systemImage: "arrow.up.forward.app")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .tint(NativePalette.primary(.light))

            HStack(spacing: 0) {
                menuIconButton(L10n.text(.refresh), image: "arrow.clockwise", disabled: model.isRefreshing) {
                    Task { await model.refresh() }
                }
                Divider().frame(height: 20)
                menuIconButton(L10n.text(.settings), image: "gearshape") {
                    AppLifecycle.shared.openSettingsWindow()
                }
                Divider().frame(height: 20)
                menuIconButton(L10n.text(.quit), image: "power") {
                    Task { await model.quit() }
                }
            }
            .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 7))
            .overlay {
                RoundedRectangle(cornerRadius: 7)
                    .stroke(.primary.opacity(0.10), lineWidth: 0.7)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(.ultraThinMaterial)
    }

    private func menuIconButton(
        _ label: String,
        image: String,
        disabled: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: image).frame(width: 29, height: 27)
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .help(label)
        .accessibilityLabel(label)
    }
}

private struct TrayProviderSurface<Content: View>: View {
    @Environment(\.colorScheme) private var colorScheme
    let content: Content

    init(@ViewBuilder content: () -> Content) { self.content = content() }

    var body: some View {
        content
            .padding(11)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
            .overlay {
                RoundedRectangle(cornerRadius: 10)
                    .stroke(NativePalette.border(colorScheme), lineWidth: 0.7)
            }
    }
}

private struct TraySubscriptionCard: View {
    let provider: TrayProviderUsageV1

    var body: some View {
        TrayProviderSurface {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    NativeProviderIcon(
                        systemPresetKey: provider.systemPresetKey,
                        productGroupId: provider.systemPresetKey ?? provider.providerName,
                        name: provider.providerName,
                        size: 27
                    )
                    VStack(alignment: .leading, spacing: 1) {
                        Text(provider.providerName)
                            .font(.system(size: 11, weight: .semibold))
                            .lineLimit(1)
                        Text(provider.subscription?.planLabel ?? L10n.text(.subscription))
                            .font(.system(size: 9))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    NativeStatusBadge(status: provider.status)
                }
                ForEach(provider.subscription?.windows ?? []) { window in
                    TrayQuotaRow(window: window)
                }
                if let resets = provider.subscription?.manualResetsRemaining {
                    Text("\(L10n.text(.resetCredits)): \(resets)")
                        .font(.system(size: 9).monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                HStack {
                    trayMetric(L10n.text(.today), NativeFormatting.count(provider.recentUsage.todayTokens))
                    trayMetric(L10n.text(.tokens), NativeFormatting.count(provider.recentUsage.totalTokens))
                    if let model = provider.recentUsage.mostUsedModel {
                        trayMetric(L10n.text(.model), model)
                    }
                }
            }
        }
    }
}

private struct TrayQuotaRow: View {
    @Environment(\.colorScheme) private var colorScheme
    let window: TrayQuotaWindowV1

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(window.kind == "five_hour" ? "5h" : "7d")
                    .fontWeight(.medium)
                Spacer()
                Text(window.remaining.map(NativeFormatting.percent) ?? L10n.text(.unavailable))
                    .fontWeight(.semibold)
                    .foregroundStyle(NativePalette.status(window.status, scheme: colorScheme))
            }
            .font(.system(size: 10).monospacedDigit())
            if let remaining = window.remaining {
                ProgressView(value: NSDecimalNumber(decimal: remaining).doubleValue, total: 100)
                    .progressViewStyle(.linear)
                    .tint(NativePalette.status(window.status, scheme: colorScheme))
                    .frame(height: 5)
            } else {
                Capsule().fill(NativePalette.recessed(colorScheme)).frame(height: 5)
            }
            HStack {
                Text(NativeFormatting.reset(window.resetsAt) ?? (window.unavailableReason ?? L10n.text(.resetTimeUnknown)))
                Spacer()
                if let burn = window.burnRatePerHour { Text("\(NativeFormatting.percent(burn))/h") }
            }
            .font(.system(size: 8).monospacedDigit())
            .foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }
}

private struct TrayBudgetCard: View {
    let budget: TrayAPIBudgetV1

    var body: some View {
        TrayProviderSurface {
            VStack(alignment: .leading, spacing: 7) {
                HStack {
                    Label(L10n.text(.budget), systemImage: "dollarsign.circle")
                        .font(.system(size: 11, weight: .semibold))
                    Spacer()
                    NativeStatusBadge(status: budget.status)
                }
                HStack {
                    trayMetric(L10n.text(.todayCost), NativeFormatting.money(budget.todayCost))
                    trayMetric(L10n.text(.dailyBudget), NativeFormatting.money(budget.dailyBudget))
                    trayMetric(L10n.text(.consumed), NativeFormatting.percent(budget.consumedPercent))
                }
                if let burn = budget.burnRatePerHour {
                    Text("\(L10n.text(.burnRate)) \(NativeFormatting.money(burn))/h")
                        .font(.system(size: 9).monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}

private struct TrayMeteredCard: View {
    let provider: TrayProviderUsageV1

    var body: some View {
        TrayProviderSurface {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    NativeProviderIcon(
                        systemPresetKey: provider.systemPresetKey,
                        productGroupId: provider.systemPresetKey ?? provider.providerName,
                        name: provider.providerName,
                        size: 27
                    )
                    VStack(alignment: .leading, spacing: 1) {
                        Text(provider.providerName)
                            .font(.system(size: 11, weight: .semibold))
                            .lineLimit(1)
                        Text(L10n.text(.metered))
                            .font(.system(size: 9))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    NativeStatusBadge(status: provider.status)
                }
                HStack {
                    trayMetric(L10n.text(.todayCost), NativeFormatting.money(provider.metered?.todayCost))
                    trayMetric(L10n.text(.rolling30Days), NativeFormatting.money(provider.metered?.rolling30DayCost))
                    trayMetric(L10n.text(.tokens), NativeFormatting.count(provider.recentUsage.totalTokens))
                }
            }
        }
    }
}

private func trayMetric(_ label: String, _ value: String) -> some View {
    VStack(alignment: .leading, spacing: 2) {
        Text(label).font(.system(size: 8)).foregroundStyle(.secondary).lineLimit(1)
        Text(value).font(.system(size: 10, weight: .semibold).monospacedDigit()).lineLimit(1)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
}
#endif
