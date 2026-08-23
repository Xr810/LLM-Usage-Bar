#if os(macOS)
import Foundation
import UsageCore

/// 路由面板的状态。四个分区共用一个 —— 它们不是四类并列设置,而是一条有依赖的启用链:
/// 没有映射 → 路由对一切请求 503;没有凭据 → 那家被跳过;没接管指针 → 一切配置都不生效;
/// 模式与分账只有在前三步成立后才有意义。
@MainActor
public final class RouterPanelModel: ObservableObject {
    @Published public private(set) var providers: [RouterProviderV1] = []
    @Published public private(set) var routes: [ModelRouteV1] = []
    @Published public private(set) var mode: RouterModeV1 = .auto
    @Published public private(set) var pointer: RouterPointerStateV1?
    @Published public private(set) var attempts: [RouterUsageSummaryV1] = []
    @Published public private(set) var isLoading = false

    /// 钉在顶部的错误。**不自动消失** —— 设计要求保存被拒时用户必须看到并处理,
    /// 会自己溜走的 toast 不够。
    @Published public var errorMessage: String?

    /// 接管指针之后的常驻横条。**Codex 不重读 config.toml**(决定 34),用户不会知道,
    /// 所以这条只有用户按「我已重启」才消失,不随时间、不随导航。
    @Published public private(set) var needsCodexRestart = false

    private let repository: any RouterRepository

    public init(repository: any RouterRepository) {
        self.repository = repository
    }

    // MARK: - 读

    public func load(range: DashboardRangeV1) async {
        isLoading = true
        defer { isLoading = false }
        do {
            async let providers = repository.providers()
            async let routes = repository.modelRoutes()
            async let mode = repository.mode()
            async let pointer = repository.pointerState()
            async let attempts = repository.recentAttempts(range: range)
            self.providers = try await providers
            self.routes = try await routes
            self.mode = try await mode
            self.pointer = try await pointer
            self.attempts = try await attempts
        } catch {
            report(error)
        }
    }

    // MARK: - 派生

    /// 按 priority 升序 —— 这个顺序就是「尝试顺序」,面板直接照它渲染。
    public var providersInTryOrder: [RouterProviderV1] {
        providers.sorted { ($0.priority, $0.id) < ($1.priority, $1.id) }
    }

    public func routes(forProvider providerId: String) -> [ModelRouteV1] {
        routes
            .filter { $0.providerId == providerId }
            .sorted { $0.logicalModel < $1.logicalModel }
    }

    /// provider 在调色板里的序号 —— 身份色按注册顺序分配,四处必须一致。
    public func identityIndex(forProvider providerId: String) -> Int {
        providersInTryOrder.firstIndex { $0.id == providerId } ?? 0
    }

    /// 一个 provider 都没有时,router 对一切请求返回 503。空态要直接说这个后果,
    /// 而不是「暂无数据」。
    public var isRouterUnusable: Bool { providers.isEmpty || routes.isEmpty }

    /// `bearer_key` 却没绑凭据 —— 这家其实用不了,要一眼看得出。
    public func isMissingCredential(_ provider: RouterProviderV1) -> Bool {
        provider.authKind == "bearer_key"
            && (provider.credentialKeyId?.isEmpty ?? true)
    }

    /// 手动模式下选中的是哪家。
    public var manuallySelectedProviderId: String? {
        if case let .manual(providerId) = mode { return providerId }
        return nil
    }

    // MARK: - 分账
    //
    // 这些数字是**下界**,不是账单:只统计经过 router 且成功抓到 usage 的请求,
    // 用户中途取消的、上游断流的、上游不发结束事件的全部缺席。UI 必须表达这一点。

    public struct AccountingTotals: Equatable, Sendable {
        public var attempts: Int64
        public var failures: Int64
        public var inputTokens: Int64
        public var outputTokens: Int64
    }

    public var accountingTotals: AccountingTotals {
        attempts.reduce(AccountingTotals(attempts: 0, failures: 0, inputTokens: 0, outputTokens: 0))
        { total, row in
            AccountingTotals(
                attempts: total.attempts + row.attempts,
                failures: total.failures + row.failures,
                inputTokens: total.inputTokens + row.inputTokens,
                outputTokens: total.outputTokens + row.outputTokens
            )
        }
    }

    public func summary(forProvider providerId: String) -> RouterUsageSummaryV1? {
        attempts.first { $0.providerId == providerId }
    }

    // MARK: - 写
    //
    // bridge 的变更命令返回变更后的新状态,所以这里直接接住,不用回查。

    public func save(provider: RouterProviderV1) async {
        do { providers = try await repository.upsertProvider(provider) } catch { report(error) }
    }

    public func delete(providerId: String) async {
        do {
            providers = try await repository.deleteProvider(id: providerId)
            routes = try await repository.modelRoutes()
        } catch {
            report(error)
        }
    }

    public func saveRoutes(providerId: String, routes newRoutes: [ModelRouteInputV1]) async {
        do {
            routes = try await repository.setModelRoutes(providerId: providerId, routes: newRoutes)
        } catch {
            report(error)
        }
    }

    public func setMode(_ newMode: RouterModeV1) async {
        do { mode = try await repository.setMode(newMode) } catch { report(error) }
    }

    /// 自动模式下的排序。priority 重排成 0,10,20…,只把真正变了的那几家写回去。
    public func moveProviders(fromOffsets source: IndexSet, toOffset destination: Int) async {
        var ordered = providersInTryOrder
        ordered.move(fromOffsets: source, toOffset: destination)

        var changed: [RouterProviderV1] = []
        for (index, provider) in ordered.enumerated() {
            let priority = Int64(index * 10)
            guard provider.priority != priority else { continue }
            var next = provider
            next.priority = priority
            changed.append(next)
        }
        // 先本地生效,拖完立刻看到结果;失败时下面的 report 会把真实状态写回来。
        providers = ordered.enumerated().map { index, provider in
            var next = provider
            next.priority = Int64(index * 10)
            return next
        }
        for provider in changed {
            do { providers = try await repository.upsertProvider(provider) } catch {
                report(error)
                return
            }
        }
    }

    /// 会真的改用户磁盘上的 `~/.codex/config.toml`。**只应由用户显式点击触发。**
    public func enablePointer() async {
        do {
            pointer = try await repository.enablePointer()
            // 成功接管才立横条 —— 失败时提示重启只会误导。
            needsCodexRestart = true
        } catch {
            report(error)
        }
    }

    public func acknowledgeCodexRestart() {
        needsCodexRestart = false
    }

    // MARK: - 错误

    private func report(_ error: Error) {
        // bridge 侧路由命令的错误是按原文回传的(不走统一脱敏),这里原样呈现 ——
        // 「非法 wire_api」这类文案正是用户据以改配置的信息。
        if case let UsageRepositoryError.remote(_, message, _) = error {
            errorMessage = message
        } else {
            errorMessage = String(describing: error)
        }
    }
}
#endif
