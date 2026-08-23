import Foundation

// MARK: - DTO
//
// 字段名与 Rust 侧 `crate::api::router` 的视图逐一对应(那边是 camelCase serde)。
// 安全面沿用 api 层的约定:**这里永远不会出现凭据明文** —— `credentialKeyId` 只是
// 指向钥匙串条目的引用,key 本身走既有的凭据通道,读不回来也不该读回来。

/// 一家上游 provider(读侧)。
public struct RouterProviderV1: Codable, Equatable, Sendable {
    public var id: String
    public var displayName: String
    public var baseUrl: String
    /// `"responses"` | `"chat_completions"`
    public var wireApi: String
    /// 越小越先试。顺序即「尝试顺序」。
    public var priority: Int64
    public var enabled: Bool
    /// `"chatgpt_oauth"` | `"bearer_key"` | `"none"`
    public var authKind: String
    /// 指向凭据条目的引用,**不是 key 本身**。
    public var credentialKeyId: String?

    public init(
        id: String,
        displayName: String,
        baseUrl: String,
        wireApi: String,
        priority: Int64,
        enabled: Bool,
        authKind: String,
        credentialKeyId: String? = nil
    ) {
        self.id = id
        self.displayName = displayName
        self.baseUrl = baseUrl
        self.wireApi = wireApi
        self.priority = priority
        self.enabled = enabled
        self.authKind = authKind
        self.credentialKeyId = credentialKeyId
    }
}

/// 一条已存在的映射(读侧,带 provider)。
public struct ModelRouteV1: Codable, Equatable, Sendable {
    public var providerId: String
    public var logicalModel: String
    public var upstreamModel: String

    public init(providerId: String, logicalModel: String, upstreamModel: String) {
        self.providerId = providerId
        self.logicalModel = logicalModel
        self.upstreamModel = upstreamModel
    }
}

/// 写侧的一条映射。不带 provider —— 写的语义是「给这个 provider 全量替换」。
public struct ModelRouteInputV1: Codable, Equatable, Sendable {
    public var logicalModel: String
    public var upstreamModel: String

    public init(logicalModel: String, upstreamModel: String) {
        self.logicalModel = logicalModel
        self.upstreamModel = upstreamModel
    }
}

/// 指针状态。三态直接映射界面上的「已接管 / 未接管 / 读不到」。
public struct RouterPointerStateV1: Codable, Equatable, Sendable {
    public enum State: String, Codable, Sendable {
        case ours
        case notOurs = "not_ours"
        case unreadable
    }

    public var state: State
    /// `notOurs` 时当前指向谁;从未配置过则为 nil。
    public var current: String?

    public init(state: State, current: String? = nil) {
        self.state = state
        self.current = current
    }
}

/// 某段时间内按 provider 汇总的路由尝试。
///
/// **这些数字是下界,不是账单。** 只统计「经过 router 且成功抓到 usage」的请求:
/// 用户中途取消的、上游断流的、上游不发结束事件的全部缺席。界面必须表达这一点。
public struct RouterUsageSummaryV1: Codable, Equatable, Sendable {
    public var providerId: String
    public var attempts: Int64
    public var failures: Int64
    public var inputTokens: Int64
    public var outputTokens: Int64

    public init(
        providerId: String,
        attempts: Int64,
        failures: Int64,
        inputTokens: Int64,
        outputTokens: Int64
    ) {
        self.providerId = providerId
        self.attempts = attempts
        self.failures = failures
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
    }
}

/// 路由模式。`auto` 或 `manual(providerId)`。
public enum RouterModeV1: Equatable, Sendable {
    case auto
    case manual(providerId: String)

    /// 线上形态是一个字符串:`"auto"` 或 `"manual:<providerId>"`。
    public init(wireValue: String) {
        if let providerId = wireValue.stripping(prefix: "manual:"), !providerId.isEmpty {
            self = .manual(providerId: providerId)
        } else {
            self = .auto
        }
    }

    public var wireValue: String {
        switch self {
        case .auto: return "auto"
        case let .manual(providerId): return "manual:\(providerId)"
        }
    }
}

extension String {
    fileprivate func stripping(prefix: String) -> String? {
        guard hasPrefix(prefix) else { return nil }
        return String(dropFirst(prefix.count))
    }
}

// MARK: - Repository

public protocol RouterRepository: Sendable {
    func providers() async throws -> [RouterProviderV1]
    func modelRoutes() async throws -> [ModelRouteV1]
    func mode() async throws -> RouterModeV1
    func pointerState() async throws -> RouterPointerStateV1
    func recentAttempts(range: DashboardRangeV1) async throws -> [RouterUsageSummaryV1]

    /// 下面五个是变更。**都返回变更后的新状态** —— bridge 侧有意如此:
    /// 面板不必再回查一次,也就没有读到旧值的窗口。
    func upsertProvider(_ provider: RouterProviderV1) async throws -> [RouterProviderV1]
    func deleteProvider(id: String) async throws -> [RouterProviderV1]
    func setModelRoutes(
        providerId: String,
        routes: [ModelRouteInputV1]
    ) async throws -> [ModelRouteV1]
    func setMode(_ mode: RouterModeV1) async throws -> RouterModeV1
    /// 会真的改用户磁盘上的 `~/.codex/config.toml`。**必须由用户显式点击触发**,
    /// 而且点完要提示重启 Codex —— Codex 不重读配置(决定 34)。这两条约束在 UI 侧,
    /// 这一层只负责转发。
    func enablePointer() async throws -> RouterPointerStateV1
}

public actor BridgeRouterRepository: RouterRepository {
    public static let supportedSchemaVersion = 1
    private let client: NativeBridgeClient

    public init(client: NativeBridgeClient = NativeBridgeClient()) {
        self.client = client
    }

    // MARK: 读

    public func providers() async throws -> [RouterProviderV1] {
        try Self.unwrap(
            await client.call(
                "listRouterProviders",
                params: nil,
                as: BridgeSchemaEnvelopeV1<[RouterProviderV1]>.self
            )
        )
    }

    public func modelRoutes() async throws -> [ModelRouteV1] {
        try Self.unwrap(
            await client.call(
                "listModelRoutes",
                params: nil,
                as: BridgeSchemaEnvelopeV1<[ModelRouteV1]>.self
            )
        )
    }

    public func mode() async throws -> RouterModeV1 {
        let raw = try Self.unwrap(
            await client.call(
                "getRouterMode",
                params: nil,
                as: BridgeSchemaEnvelopeV1<String>.self
            )
        )
        return RouterModeV1(wireValue: raw)
    }

    public func pointerState() async throws -> RouterPointerStateV1 {
        try Self.unwrap(
            await client.call(
                "inspectRouterPointer",
                params: nil,
                as: BridgeSchemaEnvelopeV1<RouterPointerStateV1>.self
            )
        )
    }

    public func recentAttempts(range: DashboardRangeV1) async throws -> [RouterUsageSummaryV1] {
        try Self.unwrap(
            await client.call(
                "recentRouterAttempts",
                params: [
                    "startAt": .integer(range.startAt),
                    "endAt": .integer(range.endAt),
                ],
                as: BridgeSchemaEnvelopeV1<[RouterUsageSummaryV1]>.self
            )
        )
    }

    // MARK: 变更

    public func upsertProvider(_ provider: RouterProviderV1) async throws -> [RouterProviderV1] {
        try Self.unwrap(
            await client.callMutation(
                "upsertRouterProvider",
                params: Self.parameters(for: provider),
                as: BridgeSchemaEnvelopeV1<[RouterProviderV1]>.self
            )
        )
    }

    public func deleteProvider(id: String) async throws -> [RouterProviderV1] {
        try Self.unwrap(
            await client.callMutation(
                "deleteRouterProvider",
                params: ["id": .string(id)],
                as: BridgeSchemaEnvelopeV1<[RouterProviderV1]>.self
            )
        )
    }

    public func setModelRoutes(
        providerId: String,
        routes: [ModelRouteInputV1]
    ) async throws -> [ModelRouteV1] {
        try Self.unwrap(
            await client.callMutation(
                "setModelRoutes",
                params: [
                    "providerId": .string(providerId),
                    "routes": .array(
                        routes.map { route in
                            .object([
                                "logicalModel": .string(route.logicalModel),
                                "upstreamModel": .string(route.upstreamModel),
                            ])
                        }
                    ),
                ],
                as: BridgeSchemaEnvelopeV1<[ModelRouteV1]>.self
            )
        )
    }

    public func setMode(_ mode: RouterModeV1) async throws -> RouterModeV1 {
        let raw = try Self.unwrap(
            await client.callMutation(
                "setRouterMode",
                params: ["mode": .string(mode.wireValue)],
                as: BridgeSchemaEnvelopeV1<String>.self
            )
        )
        return RouterModeV1(wireValue: raw)
    }

    public func enablePointer() async throws -> RouterPointerStateV1 {
        try Self.unwrap(
            await client.callMutation(
                "enableRouterPointer",
                // callMutation 的 params 不接受 nil(call 接受),空字典即可 ——
                // Rust 侧这条命令本来就不解析参数。
                params: [:],
                as: BridgeSchemaEnvelopeV1<RouterPointerStateV1>.self
            )
        )
    }

    // MARK: 内部

    /// 入参里**只有** `credentialKeyId` 这个引用,绝不会带上 key 明文 ——
    /// 与 Rust 侧 `RouterProviderInput` 的约定一致。
    private static func parameters(
        for provider: RouterProviderV1
    ) -> [String: NativeBridgeParameter] {
        var params: [String: NativeBridgeParameter] = [
            "id": .string(provider.id),
            "displayName": .string(provider.displayName),
            "baseUrl": .string(provider.baseUrl),
            "wireApi": .string(provider.wireApi),
            "priority": .integer(provider.priority),
            "enabled": .boolean(provider.enabled),
            "authKind": .string(provider.authKind),
        ]
        params["credentialKeyId"] =
            provider.credentialKeyId.map(NativeBridgeParameter.string) ?? .null
        return params
    }

    nonisolated static func unwrap<Value: Codable & Sendable>(
        _ envelope: BridgeSchemaEnvelopeV1<Value>
    ) throws -> Value {
        guard envelope.schemaVersion == Self.supportedSchemaVersion else {
            throw UsageRepositoryError.unsupportedDashboardSchema(envelope.schemaVersion)
        }
        return envelope.data
    }
}
