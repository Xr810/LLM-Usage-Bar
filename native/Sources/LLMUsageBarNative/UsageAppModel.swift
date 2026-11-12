#if os(macOS)
import AppKit
import Foundation
import UsageCore

#if NATIVE_PREVIEW
enum NativePreviewMode: String {
    case fixture
    case bridge

    static var current: Self {
        let value = ProcessInfo.processInfo.arguments
            .first { $0.hasPrefix("--preview-data=") }?
            .split(separator: "=", maxSplits: 1)
            .last
            .map(String.init)
        return value.flatMap(Self.init(rawValue:)) ?? .fixture
    }
}
#endif

@MainActor
final class UsageAppModel: ObservableObject {
    @Published private(set) var snapshot = TrayUsageSnapshotV1.unavailable()
    @Published private(set) var errorMessage: String?
    @Published private(set) var isRefreshing = false
    @Published private(set) var runtimeStatus: BridgeRuntimeStatusV1?
    @Published private(set) var providerDashboard = ProviderMonitoringDashboardV1.empty
    @Published private(set) var providerActivity: [DashboardTrendBucketV1] = []
    @Published private(set) var modelDashboard = ModelUsageDashboardV1.empty
    @Published private(set) var agentBreakdown = AgentUsageBreakdownV1.empty
    @Published private(set) var usageEvents = UsageEventPageV1.empty
    @Published private(set) var recentEventsByProvider: [String: UsageEventPageV1] = [:]
    @Published private(set) var eventsProviderId: String?
    @Published private(set) var isDashboardLoading = false
    @Published private(set) var dashboardErrorMessage: String?
    @Published private(set) var isProviderLoading = false
    @Published private(set) var isActivityLoading = false
    @Published private(set) var isModelLoading = false
    @Published private(set) var isAgentLoading = false
    @Published private(set) var providerErrorMessage: String?
    @Published private(set) var activityErrorMessage: String?
    @Published private(set) var modelErrorMessage: String?
    @Published private(set) var agentErrorMessage: String?
    @Published private(set) var nativeSettings: NativeSettingsDocumentV1?
    @Published private(set) var isSettingsLoading = false
    @Published private(set) var settingsError: SettingsRepositoryError?

    let isUsingPreviewData: Bool
    private let repository: any UsageRepository
    private let dashboardRepository: any DashboardRepository
    private let settingsRepository: any SettingsRepository
    private let bridgeProcess: BridgeProcessController
    private let usesBridge: Bool

    init(
        repository: any UsageRepository,
        dashboardRepository: any DashboardRepository,
        settingsRepository: any SettingsRepository = BridgeSettingsRepository(),
        bridgeProcess: BridgeProcessController = .shared,
        initialState: NativeInitialState? = nil,
        isUsingPreviewData: Bool = false,
        usesBridge: Bool = true,
        automaticallyLoad: Bool = true
    ) {
        self.repository = repository
        self.dashboardRepository = dashboardRepository
        self.settingsRepository = settingsRepository
        self.bridgeProcess = bridgeProcess
        self.isUsingPreviewData = isUsingPreviewData
        self.usesBridge = usesBridge
        if let initialState {
            snapshot = initialState.snapshot
            providerDashboard = initialState.providerDashboard
            providerActivity = initialState.providerActivity
            modelDashboard = initialState.modelDashboard
            agentBreakdown = initialState.agentBreakdown
            usageEvents = initialState.usageEvents
            eventsProviderId = initialState.eventsProviderId
            runtimeStatus = initialState.runtimeStatus
            errorMessage = snapshot.refreshError.map { L10n.text(.refreshFailed, detail: $0) }
        }
        if automaticallyLoad {
            Task { await loadInitialSnapshot() }
        }
    }

    convenience init() {
        #if NATIVE_PREVIEW
        switch NativePreviewMode.current {
        case .fixture:
            let state = NativePreviewFixtures.make()
            self.init(
                repository: PreviewUsageRepository(),
                dashboardRepository: PreviewDashboardRepository(),
                initialState: NativeInitialState(state),
                isUsingPreviewData: true,
                usesBridge: false
            )
        case .bridge:
            self.init(
                repository: ResilientUsageRepository(),
                dashboardRepository: BridgeDashboardRepository()
            )
        }
        #else
        self.init(
            repository: ResilientUsageRepository(),
            dashboardRepository: BridgeDashboardRepository()
        )
        #endif
    }

    var statusAccessibilityLabel: String {
        "\(L10n.text(.appName)): \(L10n.status(snapshot.status))"
    }

    func loadInitialSnapshot() async {
        if usesBridge {
            bridgeProcess.startIfNeeded()
            for _ in 0..<100 {
                if NativeBridgeClient.isSocketReachable() { break }
                try? await Task.sleep(for: .milliseconds(100))
            }
        }
        await load(refreshing: false)
        if usesBridge {
            _ = await loadSettings()
        }
    }

    func refresh() async {
        guard !isRefreshing else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        await load(refreshing: true)
    }

    func loadDashboards(range: DashboardRangeV1) async {
        isDashboardLoading = true
        dashboardErrorMessage = nil
        defer { isDashboardLoading = false }
        async let providers: Void = loadProviderDashboard(range: range)
        async let models: Void = loadModelDashboard(range: range)
        async let agents: Void = loadAgentBreakdown(range: range)
        _ = await (providers, models, agents)
        dashboardErrorMessage = providerErrorMessage ?? modelErrorMessage ?? agentErrorMessage
    }

    func loadProviderDashboard(range: DashboardRangeV1) async {
        guard !isProviderLoading else { return }
        isProviderLoading = true
        providerErrorMessage = nil
        defer { isProviderLoading = false }
        do {
            providerDashboard = try await dashboardRepository.providerDashboard(range: range)
        } catch {
            providerErrorMessage = L10n.text(.bridgeUnavailable)
        }
    }

    func loadProviderActivity(range: DashboardRangeV1) async {
        guard !isActivityLoading else { return }
        isActivityLoading = true
        activityErrorMessage = nil
        defer { isActivityLoading = false }
        do {
            providerActivity = try await dashboardRepository.providerActivity(range: range)
        } catch {
            providerActivity = []
            activityErrorMessage = L10n.text(.activityUnavailable)
        }
    }

    func loadModelDashboard(range: DashboardRangeV1) async {
        guard !isModelLoading else { return }
        isModelLoading = true
        modelErrorMessage = nil
        defer { isModelLoading = false }
        do {
            modelDashboard = try await dashboardRepository.modelDashboard(range: range)
        } catch {
            modelErrorMessage = L10n.text(.bridgeUnavailable)
        }
    }

    func loadAgentBreakdown(range: DashboardRangeV1) async {
        guard !isAgentLoading else { return }
        isAgentLoading = true
        agentErrorMessage = nil
        defer { isAgentLoading = false }
        do {
            agentBreakdown = try await dashboardRepository.agentBreakdown(range: range)
        } catch {
            agentErrorMessage = L10n.text(.bridgeUnavailable)
        }
    }

    func loadRecentEvents(providerIds: [String], range: DashboardRangeV1) async {
        for providerId in providerIds {
            do {
                recentEventsByProvider[providerId] = try await dashboardRepository.usageEvents(
                    providerId: providerId,
                    range: range,
                    page: 1,
                    pageSize: 5
                )
            } catch {
                recentEventsByProvider[providerId] = nil
            }
        }
    }

    func loadUsageEvents(
        providerId: String,
        range: DashboardRangeV1,
        page: UInt64 = 1,
        pageSize: UInt64 = 50
    ) async {
        do {
            usageEvents = try await dashboardRepository.usageEvents(
                providerId: providerId,
                range: range,
                page: page,
                pageSize: pageSize
            )
            eventsProviderId = providerId
            dashboardErrorMessage = nil
        } catch {
            usageEvents = .empty
            eventsProviderId = providerId
            dashboardErrorMessage = L10n.text(.bridgeUnavailable)
        }
    }

    @discardableResult
    func loadSettings() async -> Result<NativeSettingsDocumentV1, SettingsRepositoryError> {
        guard !isSettingsLoading else {
            if let nativeSettings { return .success(nativeSettings) }
            return .failure(.transport(.bridgeUnavailable))
        }
        isSettingsLoading = true
        settingsError = nil
        defer { isSettingsLoading = false }
        do {
            let document = try await settingsRepository.load()
            nativeSettings = document
            return .success(document)
        } catch let error as SettingsRepositoryError {
            settingsError = error
            return .failure(error)
        } catch {
            let mapped = SettingsRepositoryError.transport(.malformedResponse)
            settingsError = mapped
            return .failure(mapped)
        }
    }

    func saveSettings(
        patch: NativeSettingsPatchV1,
        expectedRevision: String
    ) async -> Result<NativeSettingsDocumentV1, SettingsRepositoryError> {
        settingsError = nil
        do {
            let document = try await settingsRepository.save(
                patch: patch,
                expectedRevision: expectedRevision
            )
            nativeSettings = document
            return .success(document)
        } catch let error as SettingsRepositoryError {
            if case let .conflict(current) = error {
                nativeSettings = current
            }
            settingsError = error
            return .failure(error)
        } catch {
            let mapped = SettingsRepositoryError.transport(.malformedResponse)
            settingsError = mapped
            return .failure(mapped)
        }
    }

    func openLegacyApplication(destination: LegacyHandoffDestinationV1) async {
        if isCanvasPreview { return }
        let handedOffInPlace = await repository.shutdownBridge(destination: destination)
        if handedOffInPlace {
            for _ in 0..<50 {
                if !FileManager.default.fileExists(
                    atPath: NativeBridgeClient.defaultSocketURL().path
                ) { break }
                try? await Task.sleep(for: .milliseconds(100))
            }
            try? await Task.sleep(for: .milliseconds(250))
        } else {
            bridgeProcess.openLegacyApplication()
        }
        NSApplication.shared.terminate(nil)
    }

    func quit() async {
        if isCanvasPreview { return }
        if usesBridge && bridgeProcess.ownsRunningProcess {
            _ = await repository.shutdownBridge(destination: nil)
        } else {
            await repository.disconnect()
        }
        NSApplication.shared.terminate(nil)
    }

    private func load(refreshing: Bool) async {
        do {
            snapshot = refreshing ? try await repository.refresh() : try await repository.snapshot()
            runtimeStatus = try? await repository.runtimeStatus()
            errorMessage = snapshot.refreshError.map { L10n.text(.refreshFailed, detail: $0) }
        } catch {
            snapshot = .unavailable()
            runtimeStatus = nil
            errorMessage = L10n.text(.bridgeUnavailable)
        }
    }

    private var isCanvasPreview: Bool {
        #if NATIVE_PREVIEW
        ProcessInfo.processInfo.environment["XCODE_RUNNING_FOR_PREVIEWS"] == "1"
        #else
        false
        #endif
    }

    #if NATIVE_PREVIEW
    static func preview(
        scenario: NativePreviewScenario = .healthy,
        now: Date = .now
    ) -> UsageAppModel {
        let state = NativePreviewFixtures.make(scenario: scenario, now: now)
        return UsageAppModel(
            repository: PreviewUsageRepository(scenario: scenario, now: now),
            dashboardRepository: PreviewDashboardRepository(scenario: scenario),
            initialState: NativeInitialState(state),
            isUsingPreviewData: true,
            usesBridge: false,
            automaticallyLoad: false
        )
    }
    #endif
}

struct NativeInitialState {
    var snapshot: TrayUsageSnapshotV1
    var providerDashboard: ProviderMonitoringDashboardV1
    var providerActivity: [DashboardTrendBucketV1]
    var modelDashboard: ModelUsageDashboardV1
    var agentBreakdown: AgentUsageBreakdownV1
    var usageEvents: UsageEventPageV1
    var eventsProviderId: String?
    var runtimeStatus: BridgeRuntimeStatusV1?

    #if NATIVE_PREVIEW
    init(_ state: NativePreviewState) {
        snapshot = state.traySnapshot
        providerDashboard = state.providerDashboard
        providerActivity = state.providerActivity
        modelDashboard = state.modelDashboard
        agentBreakdown = state.agentBreakdown
        usageEvents = state.usageEvents
        eventsProviderId = state.providerDashboard.providers.first?.id
        runtimeStatus = state.diagnostics
    }
    #endif
}

@MainActor
final class BridgeProcessController {
    static let shared = BridgeProcessController()

    private var process: Process?
    private let fileManager = FileManager.default

    var ownsRunningProcess: Bool { process?.isRunning == true }

    func startIfNeeded() {
        if NativeBridgeClient.isSocketReachable() { return }
        if process?.isRunning == true { return }
        guard let executable = bridgeExecutableURL() else { return }
        let process = Process()
        process.executableURL = executable
        process.arguments = ["--native-bridge-server"]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            self.process = process
        } catch {
            self.process = nil
        }
    }

    func openLegacyApplication() {
        guard let applicationURL = legacyApplicationURL() else { return }
        NSWorkspace.shared.openApplication(
            at: applicationURL,
            configuration: NSWorkspace.OpenConfiguration()
        )
    }

    private func bridgeExecutableURL() -> URL? {
        if let override = ProcessInfo.processInfo.environment["LLM_USAGE_BAR_BRIDGE_EXECUTABLE"],
           fileManager.isExecutableFile(atPath: override) {
            let executable = URL(fileURLWithPath: override)
            return supportsNativeBridge(executable) ? executable : nil
        }
        guard let applicationURL = legacyApplicationURL() else { return nil }
        for name in ["llm-usage-bar", "LLM Usage Bar"] {
            let candidate = applicationURL
                .appendingPathComponent("Contents/MacOS", isDirectory: true)
                .appendingPathComponent(name)
            if fileManager.isExecutableFile(atPath: candidate.path),
               supportsNativeBridge(candidate) {
                return candidate
            }
        }
        return nil
    }

    private func supportsNativeBridge(_ executable: URL) -> Bool {
        guard let bytes = try? Data(contentsOf: executable, options: .mappedIfSafe) else {
            return false
        }
        return bytes.range(of: Data("--native-bridge-server".utf8)) != nil
    }

    private func legacyApplicationURL() -> URL? {
        let application = URL(fileURLWithPath: "/Applications/LLM Usage Bar.app", isDirectory: true)
        return fileManager.fileExists(atPath: application.path) ? application : nil
    }
}
#endif
