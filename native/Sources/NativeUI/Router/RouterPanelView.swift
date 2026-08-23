#if os(macOS)
import SwiftUI
import UsageCore

/// 路由面板:**一个可滚动 pane 里从上往下四个分区**,不是四个页面、也不是步骤侧栏。
///
/// 它们是一条有依赖的启用链,位置本身就表达了先后;设置窗里再套一层导航是网页习惯。
/// 外观(主题、半透明)不在这里 —— 那是全 app 的设置,归「通用」分区(V4)。
public struct RouterPanelView: View {
    @ObservedObject private var model: RouterPanelModel
    private let appearance: NativeResolvedAppearance
    private let routerPort: Int

    private let onAddProvider: () -> Void
    private let onEditRoutes: (RouterProviderV1) -> Void
    private let onBindCredential: (RouterProviderV1) -> Void
    private let onUnbindCredential: (RouterProviderV1) -> Void
    private let onEnablePointer: () -> Void
    private let onRevealInFinder: () -> Void
    private let onModeChange: (RouterModeV1) -> Void
    private let onMoveProviders: (IndexSet, Int) -> Void

    public init(
        model: RouterPanelModel,
        appearance: NativeResolvedAppearance,
        routerPort: Int = 8788,
        onAddProvider: @escaping () -> Void = {},
        onEditRoutes: @escaping (RouterProviderV1) -> Void = { _ in },
        onBindCredential: @escaping (RouterProviderV1) -> Void = { _ in },
        onUnbindCredential: @escaping (RouterProviderV1) -> Void = { _ in },
        onEnablePointer: @escaping () -> Void = {},
        onRevealInFinder: @escaping () -> Void = {},
        onModeChange: @escaping (RouterModeV1) -> Void = { _ in },
        onMoveProviders: @escaping (IndexSet, Int) -> Void = { _, _ in }
    ) {
        self.model = model
        self.appearance = appearance
        self.routerPort = routerPort
        self.onAddProvider = onAddProvider
        self.onEditRoutes = onEditRoutes
        self.onBindCredential = onBindCredential
        self.onUnbindCredential = onUnbindCredential
        self.onEnablePointer = onEnablePointer
        self.onRevealInFinder = onRevealInFinder
        self.onModeChange = onModeChange
        self.onMoveProviders = onMoveProviders
    }

    private var theme: NativeTheme { appearance.theme }

    public var body: some View {
        ScrollView { content }
            .background(background)
            .tint(theme.accent)
            .environment(\.colorScheme, theme.appearance == .dark ? .dark : .light)
    }

    /// 内容层单独暴露,不含 ScrollView。
    ///
    /// 为什么要拆:`ImageRenderer` **渲不出 ScrollView 里的内容**(实测把整屏渲成空白,
    /// 非背景像素为 0),而无头渲图是「自己看一眼刚写的界面」的唯一通道。拆开之后
    /// 渲染器直接渲这一层,真实界面照旧带滚动。
    public var content: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.xl) {
                // 错误钉在顶部,不自动消失。
                if let message = model.errorMessage {
                    NativeErrorBanner(message: message, theme: theme) {
                        model.errorMessage = nil
                    }
                }

                ModelMappingSection(
                    model: model,
                    theme: theme,
                    onAddProvider: onAddProvider,
                    onEditRoutes: onEditRoutes
                )
                CredentialsSection(
                    model: model,
                    theme: theme,
                    onBind: onBindCredential,
                    onUnbind: onUnbindCredential
                )
                PointerTakeoverSection(
                    model: model,
                    theme: theme,
                    routerPort: routerPort,
                    onEnable: onEnablePointer,
                    onRevealInFinder: onRevealInFinder
                )
                ModeAndAccountingSection(
                    model: model,
                    theme: theme,
                    onModeChange: onModeChange,
                    onMove: onMoveProviders
                )
        }
        .padding(NativeSpacing.lg)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// 半透明关掉时落到主题底色,**不是白色** —— 纯白是「没设计过」的样子。
    @ViewBuilder
    private var background: some View {
        if appearance.usesMaterial {
            Rectangle().fill(.regularMaterial)
        } else {
            theme.ground
        }
    }
}

// MARK: - 预览

#if DEBUG
/// 预览与测试用的假数据源。**不参与正式构建。**
public actor PreviewRouterRepository: RouterRepository {
    private var storedProviders: [RouterProviderV1]
    private var storedRoutes: [ModelRouteV1]
    private var storedMode: RouterModeV1
    private var storedPointer: RouterPointerStateV1
    private var storedAttempts: [RouterUsageSummaryV1]
    /// 置位后所有调用都抛错,用来看错误条长什么样。
    private var failure: UsageRepositoryError?

    public init(
        providers: [RouterProviderV1] = PreviewRouterRepository.sampleProviders,
        routes: [ModelRouteV1] = PreviewRouterRepository.sampleRoutes,
        mode: RouterModeV1 = .auto,
        pointer: RouterPointerStateV1 = .init(state: .notOurs, current: "openai"),
        attempts: [RouterUsageSummaryV1] = PreviewRouterRepository.sampleAttempts,
        failure: UsageRepositoryError? = nil
    ) {
        self.storedProviders = providers
        self.storedRoutes = routes
        self.storedMode = mode
        self.storedPointer = pointer
        self.storedAttempts = attempts
        self.failure = failure
    }

    public static let sampleProviders: [RouterProviderV1] = [
        .init(id: "official", displayName: "官方 ChatGPT",
              baseUrl: "https://chatgpt.com/backend-api", wireApi: "responses",
              priority: 10, enabled: true, authKind: "chatgpt_oauth"),
        .init(id: "sol-relay", displayName: "Sol 中转",
              baseUrl: "https://api.sol-relay.dev/v1", wireApi: "responses",
              priority: 20, enabled: true, authKind: "bearer_key",
              credentialKeyId: "codex-router.sol"),
        .init(id: "kappa", displayName: "Kappa 中转",
              baseUrl: "https://kappa.example/v1", wireApi: "responses",
              priority: 30, enabled: true, authKind: "bearer_key"),
        .init(id: "local-llama", displayName: "本机 llama.cpp",
              baseUrl: "http://127.0.0.1:8080/v1", wireApi: "chat_completions",
              priority: 40, enabled: false, authKind: "none"),
    ]

    public static let sampleRoutes: [ModelRouteV1] = [
        .init(providerId: "official", logicalModel: "gpt-5.6", upstreamModel: "gpt-5.6"),
        .init(providerId: "sol-relay", logicalModel: "gpt-5.6",
              upstreamModel: "sol-gpt-5.6-1120"),
        .init(providerId: "sol-relay", logicalModel: "gpt-5.6-sol",
              upstreamModel: "sol-preview-1120"),
        .init(providerId: "local-llama", logicalModel: "gpt-5.6",
              upstreamModel: "qwen3-coder-30b"),
    ]

    public static let sampleAttempts: [RouterUsageSummaryV1] = [
        .init(providerId: "official", attempts: 412, failures: 7,
              inputTokens: 1_204_880, outputTokens: 318_402),
        .init(providerId: "sol-relay", attempts: 168, failures: 2,
              inputTokens: 486_110, outputTokens: 140_255),
    ]

    private func checked<T>(_ value: T) throws -> T {
        if let failure { throw failure }
        return value
    }

    public func providers() async throws -> [RouterProviderV1] { try checked(storedProviders) }
    public func modelRoutes() async throws -> [ModelRouteV1] { try checked(storedRoutes) }
    public func mode() async throws -> RouterModeV1 { try checked(storedMode) }
    public func pointerState() async throws -> RouterPointerStateV1 { try checked(storedPointer) }
    public func recentAttempts(range: DashboardRangeV1) async throws -> [RouterUsageSummaryV1] {
        try checked(storedAttempts)
    }

    public func upsertProvider(_ provider: RouterProviderV1) async throws -> [RouterProviderV1] {
        if let failure { throw failure }
        if let index = storedProviders.firstIndex(where: { $0.id == provider.id }) {
            storedProviders[index] = provider
        } else {
            storedProviders.append(provider)
        }
        return storedProviders
    }

    public func deleteProvider(id: String) async throws -> [RouterProviderV1] {
        if let failure { throw failure }
        storedProviders.removeAll { $0.id == id }
        storedRoutes.removeAll { $0.providerId == id }
        return storedProviders
    }

    public func setModelRoutes(
        providerId: String,
        routes: [ModelRouteInputV1]
    ) async throws -> [ModelRouteV1] {
        if let failure { throw failure }
        storedRoutes.removeAll { $0.providerId == providerId }
        storedRoutes.append(contentsOf: routes.map {
            ModelRouteV1(
                providerId: providerId,
                logicalModel: $0.logicalModel,
                upstreamModel: $0.upstreamModel
            )
        })
        return storedRoutes
    }

    public func setMode(_ mode: RouterModeV1) async throws -> RouterModeV1 {
        if let failure { throw failure }
        storedMode = mode
        return storedMode
    }

    public func enablePointer() async throws -> RouterPointerStateV1 {
        if let failure { throw failure }
        storedPointer = RouterPointerStateV1(state: .ours, current: nil)
        return storedPointer
    }
}

@MainActor
private func previewModel(
    repository: PreviewRouterRepository = PreviewRouterRepository()
) -> RouterPanelModel {
    let model = RouterPanelModel(repository: repository)
    Task { await model.load(range: DashboardRangeV1(startAt: 0, endAt: 1)) }
    return model
}

#Preview("默认 · 系统主题 · 材质") {
    RouterPanelView(
        model: previewModel(),
        appearance: NativeAppearanceSettings().resolve(
            appearance: .light, systemReduceTransparency: false
        )
    )
    .frame(width: 820, height: 720)
}

#Preview("Overcast · 不透明") {
    RouterPanelView(
        model: previewModel(),
        appearance: NativeAppearanceSettings(
            lightThemeId: "overcast", translucencyEnabled: false
        ).resolve(appearance: .light, systemReduceTransparency: false)
    )
    .frame(width: 820, height: 720)
}

#Preview("Ink · 深色") {
    RouterPanelView(
        model: previewModel(),
        appearance: NativeAppearanceSettings(darkThemeId: "ink", translucencyEnabled: false)
            .resolve(appearance: .dark, systemReduceTransparency: false)
    )
    .frame(width: 820, height: 720)
}

#Preview("空态 · 一个 provider 都没有") {
    RouterPanelView(
        model: previewModel(
            repository: PreviewRouterRepository(providers: [], routes: [], attempts: [])
        ),
        appearance: NativeAppearanceSettings().resolve(
            appearance: .light, systemReduceTransparency: false
        )
    )
    .frame(width: 820, height: 720)
}
#endif
#endif
