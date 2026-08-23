#if os(macOS)
import Foundation
import Testing
import UsageCore
@testable import NativeUI

private let fullRange = DashboardRangeV1(startAt: 0, endAt: 1)

@MainActor
private func loadedModel(
    repository: PreviewRouterRepository = PreviewRouterRepository()
) async -> RouterPanelModel {
    let model = RouterPanelModel(repository: repository)
    await model.load(range: fullRange)
    return model
}

// MARK: - 读与派生

@MainActor
@Test func loadPopulatesEveryDataset() async {
    let model = await loadedModel()

    #expect(model.providers.count == 4)
    #expect(model.routes.count == 4)
    #expect(model.attempts.count == 2)
    #expect(model.pointer?.state == .notOurs)
    #expect(model.mode == .auto)
    #expect(model.errorMessage == nil)
}

@MainActor
@Test func providersComeBackInTryOrder() async {
    let model = await loadedModel()

    #expect(model.providersInTryOrder.map(\.id)
        == ["official", "sol-relay", "kappa", "local-llama"])
}

@MainActor
@Test func routesAreScopedToTheirProvider() async {
    let model = await loadedModel()

    let sol = model.routes(forProvider: "sol-relay")
    #expect(sol.count == 2)
    #expect(sol.allSatisfy { $0.providerId == "sol-relay" })
    // 已停用的 provider 的映射照样读得到 —— 面板要显示它。
    #expect(model.routes(forProvider: "local-llama").count == 1)
    #expect(model.routes(forProvider: "nonexistent").isEmpty)
}

@MainActor
@Test func missingCredentialOnlyAppliesToBearerKeyProviders() async {
    let model = await loadedModel()
    let byId = Dictionary(uniqueKeysWithValues: model.providers.map { ($0.id, $0) })

    // 官方走 OAuth,没有 credentialKeyId 是正常状态,不是缺凭据。
    #expect(!model.isMissingCredential(byId["official"]!))
    // none 同理。
    #expect(!model.isMissingCredential(byId["local-llama"]!))
    // bearer_key 且已绑定。
    #expect(!model.isMissingCredential(byId["sol-relay"]!))
    // bearer_key 但没绑 —— 这家其实用不了。
    #expect(model.isMissingCredential(byId["kappa"]!))
}

@MainActor
@Test func routerIsUnusableWithoutProvidersOrRoutes() async {
    let empty = await loadedModel(
        repository: PreviewRouterRepository(providers: [], routes: [], attempts: [])
    )
    #expect(empty.isRouterUnusable)

    let noRoutes = await loadedModel(repository: PreviewRouterRepository(routes: []))
    #expect(noRoutes.isRouterUnusable)

    let healthy = await loadedModel()
    #expect(!healthy.isRouterUnusable)
}

@MainActor
@Test func accountingTotalsSumEveryRow() async {
    let model = await loadedModel()
    let totals = model.accountingTotals

    #expect(totals.attempts == 412 + 168)
    #expect(totals.failures == 7 + 2)
    #expect(totals.inputTokens == 1_204_880 + 486_110)
    #expect(totals.outputTokens == 318_402 + 140_255)
}

// MARK: - 接管与重启提示

@MainActor
@Test func enablingThePointerRaisesThePersistentRestartBanner() async {
    let model = await loadedModel()
    #expect(!model.needsCodexRestart)

    await model.enablePointer()

    #expect(model.pointer?.state == .ours)
    // 决定 34:Codex 不重读 config.toml,用户不会知道 —— 所以这条必须立起来。
    #expect(model.needsCodexRestart)

    // 而且只有用户按「我已重启」才消失。
    model.acknowledgeCodexRestart()
    #expect(!model.needsCodexRestart)
}

@MainActor
@Test func failedTakeoverDoesNotClaimARestartIsNeeded() async {
    // 接管失败还提示「请重启 Codex」会把用户带沟里 —— 他重启完发现根本没接管。
    let model = await loadedModel(
        repository: PreviewRouterRepository(
            failure: .remote(code: "router_failed", message: "写入 config.toml 失败", data: nil)
        )
    )
    await model.enablePointer()

    #expect(!model.needsCodexRestart)
    #expect(model.errorMessage == "写入 config.toml 失败")
}

// MARK: - 错误

@MainActor
@Test func remoteErrorsSurfaceTheirOriginalMessage() async {
    // bridge 侧路由命令的错误是按原文回传的,面板要原样显示 ——
    // 「非法 wire_api」这类文案正是用户据以改配置的信息,包一层就没用了。
    let model = await loadedModel(
        repository: PreviewRouterRepository(
            failure: .remote(code: "router_failed", message: "非法 wire_api: rest", data: nil)
        )
    )
    await model.save(provider: PreviewRouterRepository.sampleProviders[0])

    #expect(model.errorMessage == "非法 wire_api: rest")
}

@MainActor
@Test func errorBannerDoesNotClearItself() async {
    let model = await loadedModel()
    model.errorMessage = "保存被拒绝"

    // 再做一次成功的读,错误也不该自己消失 —— 只有用户点掉才消失。
    await model.load(range: fullRange)
    #expect(model.errorMessage == "保存被拒绝")

    model.errorMessage = nil
    #expect(model.errorMessage == nil)
}

// MARK: - 模式与排序

@MainActor
@Test func manualModeExposesTheSelectedProvider() async {
    let model = await loadedModel()
    #expect(model.manuallySelectedProviderId == nil)

    await model.setMode(.manual(providerId: "sol-relay"))
    #expect(model.manuallySelectedProviderId == "sol-relay")

    await model.setMode(.auto)
    #expect(model.manuallySelectedProviderId == nil)
}

@MainActor
@Test func movingAProviderRewritesPrioritiesInTenStepIncrements() async {
    let model = await loadedModel()

    // 把第三家(kappa)拖到最前。
    await model.moveProviders(fromOffsets: IndexSet(integer: 2), toOffset: 0)

    #expect(model.providersInTryOrder.map(\.id)
        == ["kappa", "official", "sol-relay", "local-llama"])
    #expect(model.providersInTryOrder.map(\.priority) == [0, 10, 20, 30])
}

@MainActor
@Test func identityIndexFollowsTryOrderSoColoursMatchEverywhere() async {
    let model = await loadedModel()
    #expect(model.identityIndex(forProvider: "official") == 0)
    #expect(model.identityIndex(forProvider: "local-llama") == 3)

    // 换了顺序,身份色也跟着换 —— 四处(映射表/凭据/排序/分账)始终一致。
    await model.moveProviders(fromOffsets: IndexSet(integer: 3), toOffset: 0)
    #expect(model.identityIndex(forProvider: "local-llama") == 0)
}
#endif
