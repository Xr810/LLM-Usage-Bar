#if os(macOS)
import SwiftUI
import UsageCore

/// 第一节:模型映射。
///
/// 顺序上它必须排第一 —— 没有映射,router 对一切请求 503,其余分区全是空转。
public struct ModelMappingSection: View {
    @ObservedObject private var model: RouterPanelModel
    private let theme: NativeTheme
    private let onAddProvider: () -> Void
    private let onEditRoutes: (RouterProviderV1) -> Void

    public init(
        model: RouterPanelModel,
        theme: NativeTheme,
        onAddProvider: @escaping () -> Void,
        onEditRoutes: @escaping (RouterProviderV1) -> Void
    ) {
        self.model = model
        self.theme = theme
        self.onAddProvider = onAddProvider
        self.onEditRoutes = onEditRoutes
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.sm) {
            NativeSectionHeader(
                "模型映射",
                subtitle: "Codex 发来的模型名 → 每家实际认的上游 ID。哪家有哪个模型看这里;"
                    + "先试谁、失败换谁在下面「模式与分账」里定。",
                theme: theme
            ) {
                if !model.providers.isEmpty {
                    Text("\(model.providers.count) 家 · \(model.routes.count) 条映射")
                        .font(NativeTextStyle.label)
                        .foregroundStyle(theme.textTertiary)
                }
            }

            if model.providers.isEmpty {
                emptyState
            } else {
                NativeGroup(theme: theme) {
                    ForEach(Array(model.providersInTryOrder.enumerated()), id: \.element.id) {
                        index, provider in
                        if index > 0 { NativeDivider(theme: theme, inset: 0) }
                        providerBlock(provider, index: index)
                    }
                }
                footer
            }
        }
    }

    // 空态直接说后果,不说「暂无数据」—— 用户需要知道的是「现在请求会失败」。
    private var emptyState: some View {
        NativeGroup(theme: theme) {
            VStack(alignment: .leading, spacing: NativeSpacing.sm) {
                Text("还没有配置任何 provider")
                    .font(NativeTextStyle.body)
                    .foregroundStyle(theme.textPrimary)
                Text("路由现在对所有请求返回 503。加一家 provider 并给它至少一条模型映射,"
                    + "Codex 的请求才会有地方可去。")
                    .font(NativeTextStyle.secondary)
                    .foregroundStyle(theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button("添加 Provider…", action: onAddProvider)
                    .controlSize(.regular)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(NativeSpacing.md)
        }
    }

    @ViewBuilder
    private func providerBlock(_ provider: RouterProviderV1, index: Int) -> some View {
        // 停用状态要盖住**整块**(表头 + 它的映射行)。只暗表头的话,底下的映射
        // 看起来仍在生效 —— 那正是这一屏最不该给错的信息。
        providerBlockContent(provider, index: index)
            .opacity(provider.enabled ? 1 : 0.55)
    }

    @ViewBuilder
    private func providerBlockContent(_ provider: RouterProviderV1, index: Int) -> some View {
        let providerRoutes = model.routes(forProvider: provider.id)

        NativeRow(theme: theme) {
            HStack(spacing: NativeSpacing.xs) {
                NativeIdentityDot(index: index, appearance: theme.appearance)
                NativeRowLabel(
                    provider.displayName,
                    detail: "\(provider.baseUrl) · \(provider.authKind)",
                    theme: theme
                )
            }
        } trailing: {
            HStack(spacing: NativeSpacing.xs) {
                statusBadge(for: provider)
                Button("编辑…") { onEditRoutes(provider) }
                    .controlSize(.small)
            }
        }
        .accessibilityElement(children: .combine)

        if providerRoutes.isEmpty {
            NativeDivider(theme: theme)
            NativeRow(theme: theme) {
                Text("这家还没有任何映射,会被直接跳过")
                    .font(NativeTextStyle.secondary)
                    .foregroundStyle(theme.textTertiary)
                    .padding(.leading, NativeSpacing.md)
            } trailing: {
                Button("编辑映射…") { onEditRoutes(provider) }
                    .controlSize(.small)
            }
        } else {
            ForEach(providerRoutes, id: \.logicalModel) { route in
                NativeDivider(theme: theme)
                routeRow(route)
            }
        }
    }

    private func routeRow(_ route: ModelRouteV1) -> some View {
        NativeRow(theme: theme) {
            HStack(spacing: NativeSpacing.sm) {
                Text(route.logicalModel)
                    .font(NativeTextStyle.tabularNumber)
                    .foregroundStyle(theme.textPrimary)
                    .frame(width: 148, alignment: .leading)
                Image(systemName: "arrow.right")
                    .font(NativeTextStyle.footnote)
                    .foregroundStyle(theme.textTertiary)
                Text(route.upstreamModel)
                    .font(NativeTextStyle.tabularNumber)
                    .foregroundStyle(theme.textSecondary)
            }
            .padding(.leading, NativeSpacing.md)
        } trailing: {
            EmptyView()
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(route.logicalModel) 映射到 \(route.upstreamModel)")
    }

    private func statusBadge(for provider: RouterProviderV1) -> some View {
        Group {
            if !provider.enabled {
                NativeStatusBadge(.inactive("已停用"), theme: theme)
            } else if model.isMissingCredential(provider) {
                NativeStatusBadge(.warning("缺少凭据"), theme: theme)
            } else if provider.authKind == "chatgpt_oauth" {
                NativeStatusBadge(.ok("已登录"), theme: theme)
            } else {
                NativeStatusBadge(.ok("凭据已绑定"), theme: theme)
            }
        }
    }

    private var footer: some View {
        HStack(spacing: NativeSpacing.xs) {
            Button("添加 Provider…", action: onAddProvider)
                .controlSize(.small)
            Spacer()
            Text("顺序在「模式与分账」里调整")
                .font(NativeTextStyle.footnote)
                .foregroundStyle(theme.textTertiary)
        }
        .padding(.horizontal, NativeSpacing.xxs)
    }
}
#endif
