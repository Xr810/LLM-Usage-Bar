#if os(macOS)
import SwiftUI
import UsageCore

struct NativeSettingsView: View {
    enum Pane: String, CaseIterable, Identifiable {
        case general
        case providers
        case diagnostics

        var id: String { rawValue }

        var title: String {
            switch self {
            case .general: L10n.text(.general)
            case .providers: L10n.text(.providers)
            case .diagnostics: L10n.text(.diagnostics)
            }
        }

        var systemImage: String {
            switch self {
            case .general: "gearshape"
            case .providers: "server.rack"
            case .diagnostics: "stethoscope"
            }
        }
    }

    @ObservedObject var model: UsageAppModel
    @State private var pane: Pane

    init(model: UsageAppModel, initialPane: Pane = .general) {
        self.model = model
        _pane = State(initialValue: initialPane)
    }

    var body: some View {
        ZStack {
            NativeWindowBackground()
            HStack(spacing: 0) {
                settingsSidebar
                Divider().opacity(0.65)
                settingsContent
            }
        }
        .frame(minWidth: 720, minHeight: 500)
        .controlSize(.small)
        .onAppear { AppLifecycle.shared.enterForegroundMode() }
    }

    private var settingsSidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 9) {
                Image(systemName: "chart.bar.xaxis")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 26, height: 26)
                    .background(NativePalette.primary(.light), in: RoundedRectangle(cornerRadius: 7))
                VStack(alignment: .leading, spacing: 1) {
                    Text(L10n.text(.appName))
                        .font(.system(size: 12, weight: .semibold))
                    Text(L10n.text(.settings))
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 17)
            .padding(.bottom, 15)

            VStack(spacing: 4) {
                ForEach(Pane.allCases) { item in
                    Button {
                        pane = item
                    } label: {
                        HStack(spacing: 9) {
                            Image(systemName: item.systemImage)
                                .frame(width: 16)
                            Text(item.title)
                            Spacer()
                        }
                        .font(.system(size: 11, weight: pane == item ? .semibold : .regular))
                        .foregroundStyle(pane == item ? .primary : .secondary)
                        .padding(.horizontal, 10)
                        .frame(height: 32)
                        .background {
                            if pane == item {
                                RoundedRectangle(cornerRadius: 7, style: .continuous)
                                    .fill(.selection.opacity(0.72))
                            }
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(pane == item ? .isSelected : [])
                }
            }
            .padding(.horizontal, 8)

            Spacer()

            #if NATIVE_PREVIEW
            if model.isUsingPreviewData {
                Label(L10n.text(.sampleData), systemImage: "testtube.2")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.secondary)
                    .padding(14)
            }
            #endif
        }
        .frame(width: 190)
        .background(.ultraThinMaterial)
    }

    private var settingsContent: some View {
        VStack(spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(pane.title)
                        .font(.system(size: 19, weight: .semibold))
                    Text(L10n.text(.settingsDescription))
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                NativeBadge(text: L10n.text(.previewReadOnly))
            }
            .padding(.horizontal, 22)
            .padding(.vertical, 15)
            .background(.ultraThinMaterial)

            Divider().opacity(0.55)

            ScrollView {
                Group {
                    switch pane {
                    case .general: generalPane
                    case .providers: providersPane
                    case .diagnostics: diagnosticsPane
                    }
                }
                .padding(22)
            }
        }
    }

    private var generalPane: some View {
        VStack(alignment: .leading, spacing: 14) {
            NativeCard {
                VStack(alignment: .leading, spacing: 12) {
                    settingsCardTitle(L10n.text(.configuration), systemImage: "lock.shield")
                    Text(L10n.text(.readOnlyNotice))
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Divider()
                    settingsRow(L10n.text(.status), value: L10n.status(model.snapshot.status))
                    settingsRow(L10n.text(.lastUpdated), value: NativeFormatting.relative(model.snapshot.generatedAt))
                    settingsRow(L10n.text(.accountCount), value: model.providerDashboard.providers.count.formatted())
                }
            }
            legacyButton
        }
    }

    private var providersPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            if model.providerDashboard.providers.isEmpty {
                NativeCard {
                    DashboardEmptyState(title: L10n.text(.noProviders), systemImage: "server.rack")
                        .frame(maxWidth: .infinity, minHeight: 140)
                }
            } else {
                ForEach(model.providerDashboard.providers) { row in
                    NativeCard {
                        HStack(spacing: 12) {
                            NativeProviderIcon(
                                systemPresetKey: row.provider.systemPresetKey,
                                productGroupId: row.provider.productGroupId,
                                name: row.provider.name,
                                size: 34
                            )
                            VStack(alignment: .leading, spacing: 4) {
                                HStack(spacing: 6) {
                                    Text(row.provider.name)
                                        .font(.system(size: 12, weight: .semibold))
                                    NativeBadge(text: row.provider.billingKind == .subscription
                                        ? L10n.text(.subscription)
                                        : L10n.text(.metered))
                                    if row.quotaFetchState?.stale == true {
                                        NativeBadge(text: L10n.text(.stale), status: .yellow)
                                    }
                                }
                                Text(NativeFormatting.sourceLabels(row.tokenSources) ?? L10n.text(.unavailable))
                                    .font(.system(size: 10))
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            VStack(alignment: .trailing, spacing: 3) {
                                Text(NativeFormatting.count(row.totalTokens))
                                    .font(.system(size: 12, weight: .semibold).monospacedDigit())
                                Text("\(row.eventCount.formatted()) \(L10n.text(.calls))")
                                    .font(.system(size: 9).monospacedDigit())
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            legacyButton
        }
    }

    private var diagnosticsPane: some View {
        VStack(alignment: .leading, spacing: 14) {
            NativeCard {
                VStack(alignment: .leading, spacing: 12) {
                    settingsCardTitle(L10n.text(.connection), systemImage: "point.3.connected.trianglepath.dotted")
                    settingsRow(
                        L10n.text(.connection),
                        value: model.runtimeStatus == nil ? L10n.text(.disconnected) : L10n.text(.connected)
                    )
                    settingsRow(L10n.text(.databaseOwner), value: model.runtimeStatus?.databaseOwner ?? L10n.text(.unavailable))
                    settingsRow(L10n.text(.schedulerOwner), value: model.runtimeStatus?.schedulerOwner ?? L10n.text(.unavailable))
                    settingsRow(L10n.text(.clients), value: model.runtimeStatus?.clientCount.formatted() ?? L10n.text(.unavailable))
                    settingsRow(L10n.text(.stale), value: model.snapshot.stale ? "Yes" : "No")
                    settingsRow(L10n.text(.refreshFailed), value: model.snapshot.refreshError ?? "—")
                }
            }
            legacyButton
        }
    }

    private var legacyButton: some View {
        Button {
            Task { await model.openLegacyApplication(destination: .settings) }
        } label: {
            Label(L10n.text(.currentAppSettings), systemImage: "arrow.up.forward.app")
        }
        .buttonStyle(.borderedProminent)
        .frame(maxWidth: .infinity, alignment: .trailing)
    }

    private func settingsCardTitle(_ title: String, systemImage: String) -> some View {
        Label(title, systemImage: systemImage)
            .font(.system(size: 12, weight: .semibold))
    }

    private func settingsRow(_ label: String, value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 16) {
            Text(label)
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
            Spacer()
            Text(value)
                .font(.system(size: 10).monospacedDigit())
                .multilineTextAlignment(.trailing)
                .textSelection(.enabled)
        }
    }
}
#endif
