#if os(macOS)
import SwiftUI
import UsageCore

/// 第三节:接管 Codex。
///
/// 这一屏有两条不能妥协的约束:
/// 1. 点「接管」会**真的改用户磁盘上的 `~/.codex/config.toml`**。所以它不是一个
///    不起眼的 toggle —— 先把将要写入的原文摊开、把备份承诺讲清楚,再给一个普通按钮。
///    「知道自己在授权什么」靠看得见,不靠警告色。
/// 2. 点完**必须提示重启 Codex**(决定 34):Codex 不重读 config.toml,而用户不会知道。
///    所以那是一条常驻横条,不是会自己溜走的 toast。
public struct PointerTakeoverSection: View {
    @ObservedObject private var model: RouterPanelModel
    private let theme: NativeTheme
    private let routerPort: Int
    private let onEnable: () -> Void
    private let onRevealInFinder: () -> Void

    public init(
        model: RouterPanelModel,
        theme: NativeTheme,
        routerPort: Int = 8788,
        onEnable: @escaping () -> Void,
        onRevealInFinder: @escaping () -> Void
    ) {
        self.model = model
        self.theme = theme
        self.routerPort = routerPort
        self.onEnable = onEnable
        self.onRevealInFinder = onRevealInFinder
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.sm) {
            NativeSectionHeader("接管 Codex", theme: theme) {
                stateBadge
            }

            if model.needsCodexRestart {
                restartBanner
            }

            NativeGroup(theme: theme) {
                VStack(alignment: .leading, spacing: NativeSpacing.sm) {
                    headline
                    diffPreview
                    Text("原文件先备份为 config.toml.bak,其余内容不动。随时可以在这里还原。")
                        .font(NativeTextStyle.label)
                        .foregroundStyle(theme.textSecondary)
                    actions
                }
                .padding(NativeSpacing.md)
            }
        }
    }

    private var pointerState: RouterPointerStateV1.State? { model.pointer?.state }

    private var stateBadge: some View {
        Group {
            switch pointerState {
            case .ours:
                NativeStatusBadge(.ok("已接管"), theme: theme)
            case .notOurs:
                NativeStatusBadge(.inactive("未接管"), theme: theme)
            case .unreadable:
                NativeStatusBadge(.warning("读不到配置"), theme: theme)
            case nil:
                NativeStatusBadge(.inactive("检查中"), theme: theme)
            }
        }
    }

    @ViewBuilder
    private var headline: some View {
        switch pointerState {
        case .ours:
            Text("Codex 的请求正在经过本地路由。")
                .font(NativeTextStyle.body)
                .foregroundStyle(theme.textPrimary)
        case .notOurs:
            let current = model.pointer?.current
            Text(current.map { "Codex 目前直接连 \($0)。这里的映射、顺序和分账都还没有生效。" }
                ?? "Codex 还没有配置 model_provider。这里的映射、顺序和分账都还没有生效。")
                .font(NativeTextStyle.body)
                .foregroundStyle(theme.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
        case .unreadable:
            Text("读不到 ~/.codex/config.toml。可能是权限问题,也可能文件不存在 —— "
                + "接管会创建它。")
                .font(NativeTextStyle.body)
                .foregroundStyle(theme.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
        case nil:
            Text("正在读取 Codex 的配置…")
                .font(NativeTextStyle.body)
                .foregroundStyle(theme.textSecondary)
        }
    }

    /// 把将要写入的原文摊开。删除行带删除线,新增行带变更条 ——
    /// 这比任何一句「我们会修改你的配置」都清楚。
    private var diffPreview: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("接管会向 ~/.codex/config.toml 写入这几行")
                .font(NativeTextStyle.label)
                .foregroundStyle(theme.textSecondary)
                .padding(.horizontal, NativeSpacing.sm)
                .padding(.vertical, NativeSpacing.xs)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(theme.selection)

            VStack(alignment: .leading, spacing: 2) {
                if let current = model.pointer?.current {
                    diffLine("model_provider = \"\(current)\"", kind: .removed)
                }
                diffLine("model_provider = \"codex-router\"", kind: .added)
                diffLine("[model_providers.codex-router]", kind: .added)
                diffLine("base_url = \"http://127.0.0.1:\(routerPort)/v1\"", kind: .added)
                diffLine("wire_api = \"responses\"", kind: .added)
            }
            .padding(NativeSpacing.sm)
        }
        .background(theme.ground.opacity(0.6))
        .clipShape(NativeRadius.shape(NativeRadius.medium))
        .overlay(
            NativeRadius.shape(NativeRadius.medium)
                .strokeBorder(theme.hairline, lineWidth: 0.5)
        )
        .accessibilityElement(children: .combine)
        .accessibilityLabel("接管会写入的配置内容预览")
    }

    private enum DiffKind { case added, removed }

    private func diffLine(_ text: String, kind: DiffKind) -> some View {
        HStack(spacing: NativeSpacing.xs) {
            Rectangle()
                .fill(
                    kind == .added
                        ? NativeStatusColor.ok.resolved(for: theme.appearance)
                        : Color.clear
                )
                .frame(width: 2)
            Text(text)
                .font(NativeTextStyle.tabularNumber)
                .foregroundStyle(kind == .added ? theme.textPrimary : theme.textTertiary)
                .strikethrough(kind == .removed, color: theme.textTertiary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var actions: some View {
        HStack(spacing: NativeSpacing.sm) {
            if pointerState == .ours {
                Text("已接管。改回去只需在 Codex 里把 model_provider 改回原值。")
                    .font(NativeTextStyle.label)
                    .foregroundStyle(theme.textSecondary)
            } else {
                Button("接管路由…", action: onEnable)
                    .controlSize(.regular)
                    .disabled(model.isRouterUnusable)
                Text(model.isRouterUnusable
                    ? "先配好至少一条模型映射,否则接管之后请求会全部 503。"
                    : "会修改磁盘上的 ~/.codex/config.toml")
                    .font(NativeTextStyle.label)
                    .foregroundStyle(theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer()
            Button("在访达中显示", action: onRevealInFinder)
                .controlSize(.small)
        }
    }

    /// 常驻,直到用户按「我已重启」。不随时间消失、不随导航消失。
    private var restartBanner: some View {
        HStack(alignment: .firstTextBaseline, spacing: NativeSpacing.xs) {
            Image(systemName: NativeStatusColor.Symbol.warning)
                .foregroundStyle(NativeStatusColor.warning.resolved(for: theme.appearance))
            VStack(alignment: .leading, spacing: 2) {
                Text("请重启 Codex")
                    .font(NativeTextStyle.body)
                    .foregroundStyle(theme.textPrimary)
                Text("Codex 不会重新读取 config.toml。在它重启之前,请求仍然直连原来那家,"
                    + "也不会被记账。")
                    .font(NativeTextStyle.label)
                    .foregroundStyle(theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: NativeSpacing.xs)
            Button("我已重启") { model.acknowledgeCodexRestart() }
                .controlSize(.small)
        }
        .padding(NativeSpacing.sm)
        .background(NativeStatusColor.warning.resolved(for: theme.appearance).opacity(0.12))
        .clipShape(NativeRadius.shape(NativeRadius.medium))
        .accessibilityElement(children: .combine)
    }
}
#endif
