#if os(macOS)
import SwiftUI
import UsageCore

enum DashboardDimension: String, CaseIterable, Identifiable {
    case providers
    case models
    case agents

    var id: String { rawValue }

    var title: String {
        switch self {
        case .providers: L10n.text(.providerMonitoring)
        case .models: L10n.text(.byModel)
        case .agents: L10n.text(.byAgent)
        }
    }

    var description: String {
        switch self {
        case .providers: L10n.text(.providerMonitoringDescription)
        case .models: L10n.text(.byModelDescription)
        case .agents: L10n.text(.byAgentDescription)
        }
    }

    var tabTitle: String {
        switch self {
        case .providers: L10n.text(.providers)
        case .models: L10n.text(.models)
        case .agents: L10n.text(.agents)
        }
    }
}

struct MainWindowView: View {
    @ObservedObject var model: UsageAppModel
    @SceneStorage("main.dashboard.dimension") private var dimensionRaw = DashboardDimension.providers.rawValue
    @State private var rangeSelection = UsageRangeSelectionV1(preset: .thirtyDays)

    private var dimension: DashboardDimension {
        DashboardDimension(rawValue: dimensionRaw) ?? .providers
    }

    private var range: DashboardRangeV1 {
        DashboardRangeResolver.resolve(rangeSelection)
    }

    var body: some View {
        ZStack {
            NativeWindowBackground()
            VStack(spacing: 0) {
                dashboardHeader
                Divider().opacity(0.55)
                dashboardContent
            }
        }
        .toolbar { appToolbar }
        .controlSize(.small)
        .task { await loadSelectedDimension(includeActivity: true) }
        .onChange(of: dimensionRaw) { _ in
            Task { await loadSelectedDimension(includeActivity: false) }
        }
        .onChange(of: rangeSelection) { _ in
            Task { await loadSelectedDimension(includeActivity: false) }
        }
    }

    private var dashboardHeader: some View {
        HStack(alignment: .top, spacing: 20) {
            VStack(alignment: .leading, spacing: 3) {
                Text(dimension.title)
                    .font(.system(size: 18, weight: .semibold))
                    .tracking(-0.25)
                Text(dimension.description)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            Spacer(minLength: 16)
            Picker(L10n.text(.configuration), selection: $dimensionRaw) {
                ForEach(DashboardDimension.allCases) { item in
                    Text(item.tabTitle).tag(item.rawValue)
                }
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            .frame(width: 270)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 13)
        .background(.ultraThinMaterial)
    }

    @ViewBuilder
    private var dashboardContent: some View {
        switch dimension {
        case .providers:
            ProviderMonitoringView(
                model: model,
                range: range,
                rangeLabel: rangeLabel,
                rangeSelection: $rangeSelection
            )
        case .models:
            ModelBreakdownView(
                dashboard: model.modelDashboard,
                isLoading: model.isModelLoading,
                errorMessage: model.modelErrorMessage,
                rangeLabel: rangeLabel,
                rangeSelection: $rangeSelection
            )
        case .agents:
            AgentBreakdownView(
                dashboard: model.agentBreakdown,
                isLoading: model.isAgentLoading,
                errorMessage: model.agentErrorMessage,
                rangeLabel: rangeLabel,
                rangeSelection: $rangeSelection
            )
        }
    }

    @ToolbarContentBuilder
    private var appToolbar: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            HStack(spacing: 7) {
                Image(systemName: "chart.bar.xaxis")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 23, height: 23)
                    .background(NativePalette.primary(.light), in: RoundedRectangle(cornerRadius: 6))
                Text(L10n.text(.appName))
                    .font(.system(size: 13, weight: .semibold))
            }
        }
        #if NATIVE_PREVIEW
        if model.isUsingPreviewData {
            ToolbarItem {
                NativeBadge(text: L10n.text(.sampleData))
            }
        }
        #endif
        ToolbarItemGroup {
            Button {
                Task {
                    await model.refresh()
                    await loadSelectedDimension(includeActivity: dimension == .providers)
                }
            } label: {
                Label(L10n.text(.refresh), systemImage: "arrow.clockwise")
            }
            .keyboardShortcut("r", modifiers: .command)
            .disabled(model.isRefreshing)

            Button {
                AppLifecycle.shared.openSettingsWindow()
            } label: {
                Label(L10n.text(.settings), systemImage: "gearshape")
            }
            .keyboardShortcut(",", modifiers: .command)
        }
    }

    private var rangeLabel: String {
        switch rangeSelection.preset {
        case .today: return L10n.text(.today)
        case .sevenDays: return L10n.text(.sevenDays)
        case .thirtyDays: return L10n.text(.thirtyDays)
        case .oneYear: return L10n.text(.oneYear)
        case .custom:
            let start = Date(timeIntervalSince1970: TimeInterval(range.startAt))
            let end = Date(timeIntervalSince1970: TimeInterval(range.endAt))
            return "\(start.formatted(date: .abbreviated, time: .omitted)) – \(end.formatted(date: .abbreviated, time: .omitted))"
        }
    }

    private func loadSelectedDimension(includeActivity: Bool) async {
        switch dimension {
        case .providers:
            async let dashboard: Void = model.loadProviderDashboard(range: range)
            if includeActivity || model.providerActivity.isEmpty {
                async let activity: Void = model.loadProviderActivity(
                    range: DashboardRangeResolver.providerActivity()
                )
                _ = await (dashboard, activity)
            } else {
                await dashboard
            }
            let meteredIds = model.providerDashboard.providers
                .filter { $0.provider.billingKind == .metered }
                .map(\.id)
            await model.loadRecentEvents(providerIds: meteredIds, range: range)
        case .models:
            await model.loadModelDashboard(range: range)
        case .agents:
            await model.loadAgentBreakdown(range: range)
        }
    }
}

struct UsageRangeControl: View {
    @Binding var selection: UsageRangeSelectionV1
    @State private var showingCustomRange = false
    @State private var draftStart = Date.now.addingTimeInterval(-7 * 24 * 60 * 60)
    @State private var draftEnd = Date.now
    @State private var liveEnd = false

    var body: some View {
        HStack(spacing: 7) {
            Picker(L10n.text(.timeRange), selection: presetBinding) {
                Text(L10n.text(.today)).tag(UsageRangePresetV1.today)
                Text(L10n.text(.sevenDays)).tag(UsageRangePresetV1.sevenDays)
                Text(L10n.text(.thirtyDays)).tag(UsageRangePresetV1.thirtyDays)
                Text(L10n.text(.oneYear)).tag(UsageRangePresetV1.oneYear)
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            .frame(width: 265)

            Button {
                seedDraft()
                showingCustomRange.toggle()
            } label: {
                Label(L10n.text(.customRange), systemImage: "calendar")
                    .labelStyle(.iconOnly)
            }
            .help(L10n.text(.customRange))
            .popover(isPresented: $showingCustomRange, arrowEdge: .bottom) {
                customRangePopover
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(L10n.text(.timeRange))
    }

    private var presetBinding: Binding<UsageRangePresetV1> {
        Binding(
            get: { selection.preset == .custom ? .thirtyDays : selection.preset },
            set: { selection = UsageRangeSelectionV1(preset: $0) }
        )
    }

    private var customRangePopover: some View {
        VStack(alignment: .leading, spacing: 13) {
            Text(L10n.text(.customRange))
                .font(.headline)
            DatePicker(L10n.text(.startDate), selection: $draftStart)
            DatePicker(L10n.text(.endDate), selection: $draftEnd)
                .disabled(liveEnd)
            Toggle(L10n.text(.liveEnd), isOn: $liveEnd)
            HStack {
                Spacer()
                Button(L10n.text(.cancel)) { showingCustomRange = false }
                Button(L10n.text(.apply)) {
                    selection = UsageRangeSelectionV1(
                        preset: .custom,
                        customStartAt: Int64(draftStart.timeIntervalSince1970),
                        customEndAt: Int64(draftEnd.timeIntervalSince1970),
                        liveEndTime: liveEnd
                    )
                    showingCustomRange = false
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(16)
        .frame(width: 330)
    }

    private func seedDraft() {
        let now = Date.now
        draftStart = selection.customStartAt.map { Date(timeIntervalSince1970: TimeInterval($0)) }
            ?? now.addingTimeInterval(-7 * 24 * 60 * 60)
        draftEnd = selection.customEndAt.map { Date(timeIntervalSince1970: TimeInterval($0)) } ?? now
        liveEnd = selection.liveEndTime
    }
}
#endif
