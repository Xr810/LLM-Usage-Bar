import Foundation
import Testing
@testable import UsageCore

/// `router-contract-v1.json` 是 Rust 与 Swift 共用的一份 fixture:
/// Rust 侧 `native_bridge::tests::router_contract_fixture_matches_the_real_rust_views`
/// 断言真实视图**序列化出来**就是这份 JSON,这里断言 Swift 的 DTO 能把它**解回来**。
/// 两边都不改这个文件,才说明协议没有跟后端脱节。
private struct RouterFixtureV1: Decodable {
    let providers: BridgeSchemaEnvelopeV1<[RouterProviderV1]>
    let routes: BridgeSchemaEnvelopeV1<[ModelRouteV1]>
    let pointer: BridgeSchemaEnvelopeV1<RouterPointerStateV1>
    let attempts: BridgeSchemaEnvelopeV1<[RouterUsageSummaryV1]>
}

private func routerFixture() throws -> RouterFixtureV1 {
    let data = try Data(contentsOf: Bundle.module.url(
        forResource: "router-contract-v1",
        withExtension: "json",
        subdirectory: "Fixtures"
    )!)
    return try JSONDecoder().decode(RouterFixtureV1.self, from: data)
}

@Test func routerProvidersDecodeIncludingDisabledAndMissingCredential() throws {
    let providers = try routerFixture().providers

    #expect(providers.schemaVersion == 1)
    #expect(providers.data.count == 3)

    // 官方走 OAuth,没有凭据引用是正常状态,不是「缺凭据」。
    let official = providers.data[0]
    #expect(official.authKind == "chatgpt_oauth")
    #expect(official.credentialKeyId == nil)
    #expect(official.enabled)

    // 中转要 bearer key,凭据只以引用形态出现 —— 明文永远不该到这一层。
    let relay = providers.data[1]
    #expect(relay.authKind == "bearer_key")
    #expect(relay.credentialKeyId == "keychain-entry-1")

    // 已停用的 provider 照样要出现在读结果里,面板需要显示它。
    let local = providers.data[2]
    #expect(!local.enabled)
    #expect(local.wireApi == "chat_completions")

    // priority 升序就是「尝试顺序」,面板直接照这个顺序渲染。
    #expect(providers.data.map(\.priority) == [10, 20, 30])
}

@Test func routerModelRoutesCarryProviderAndSurviveDisabledProviders() throws {
    let routes = try routerFixture().routes

    #expect(routes.schemaVersion == 1)
    #expect(routes.data.count == 3)
    // 已停用的 local-llama 的映射也在 —— 这正是 listModelRoutes 与路由决策路径
    // (只看 enabled)的关键差别。
    #expect(routes.data.contains(
        ModelRouteV1(
            providerId: "local-llama",
            logicalModel: "gpt-5.6",
            upstreamModel: "qwen3-coder-30b"
        )
    ))
}

@Test func routerPointerStateDecodesSnakeCaseVariant() throws {
    let pointer = try routerFixture().pointer

    // 线上是 snake_case 的 "not_ours",Swift 侧枚举必须认得。
    #expect(pointer.data.state == .notOurs)
    #expect(pointer.data.current == "openai")
}

@Test func routerAttemptsAreLowerBoundCounters() throws {
    let attempts = try routerFixture().attempts

    #expect(attempts.data.count == 2)
    #expect(attempts.data[0].attempts == 412)
    #expect(attempts.data[0].failures == 7)
    #expect(attempts.data[0].inputTokens == 1_204_880)
    // 失败次数不从尝试次数里扣 —— 两个是独立计数。
    #expect(attempts.data[0].attempts > attempts.data[0].failures)
}

@Test func routerModeRoundTripsThroughItsWireString() {
    #expect(RouterModeV1(wireValue: "auto") == .auto)
    #expect(RouterModeV1(wireValue: "manual:sol-relay") == .manual(providerId: "sol-relay"))
    #expect(RouterModeV1.manual(providerId: "sol-relay").wireValue == "manual:sol-relay")
    #expect(RouterModeV1.auto.wireValue == "auto")

    // 垃圾值与空 provider 都退回 auto —— 与 Rust 读侧的宽容一致。
    #expect(RouterModeV1(wireValue: "manual:") == .auto)
    #expect(RouterModeV1(wireValue: "nonsense") == .auto)
}
