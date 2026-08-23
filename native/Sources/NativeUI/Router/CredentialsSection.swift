#if os(macOS)
import SwiftUI
import UsageCore

/// 第二节:凭据。
///
/// **安全面决定了这一屏的形态:key 一旦存进钥匙串就再也读不回来。**
/// 后端的输入输出结构体里永远不会出现凭据明文,router 只认 `credentialKeyId` 这个引用。
/// 所以这里没有密码框、没有眼睛图标 —— 能做的只有「已绑定 / 替换 / 解绑」。
public struct CredentialsSection: View {
    @ObservedObject private var model: RouterPanelModel
    private let theme: NativeTheme
    private let onBind: (RouterProviderV1) -> Void
    private let onUnbind: (RouterProviderV1) -> Void

    public init(
        model: RouterPanelModel,
        theme: NativeTheme,
        onBind: @escaping (RouterProviderV1) -> Void,
        onUnbind: @escaping (RouterProviderV1) -> Void
    ) {
        self.model = model
        self.theme = theme
        self.onBind = onBind
        self.onUnbind = onUnbind
    }

    private var missingCount: Int {
        model.providers.filter { model.isMissingCredential($0) }.count
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.sm) {
            NativeSectionHeader(
                "凭据",
                subtitle: "存进钥匙串之后 key 不再可读 —— 这里只显示它绑在哪,不显示内容。",
                theme: theme
            ) {
                if missingCount > 0 {
                    NativeStatusBadge(.warning("\(missingCount) 家待绑定"), theme: theme)
                }
            }

            if model.providers.isEmpty {
                NativeGroup(theme: theme) {
                    NativeRow(theme: theme) {
                        Text("先在上面添加 provider,再回来绑定凭据。")
                            .font(NativeTextStyle.secondary)
                            .foregroundStyle(theme.textTertiary)
                    } trailing: { EmptyView() }
                }
            } else {
                NativeGroup(theme: theme) {
                    ForEach(Array(model.providersInTryOrder.enumerated()), id: \.element.id) {
                        index, provider in
                        if index > 0 { NativeDivider(theme: theme) }
                        credentialRow(provider, index: index)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func credentialRow(_ provider: RouterProviderV1, index: Int) -> some View {
        NativeRow(theme: theme) {
            HStack(spacing: NativeSpacing.xs) {
                NativeIdentityDot(index: index, appearance: theme.appearance)
                NativeRowLabel(provider.displayName, detail: detail(for: provider), theme: theme)
            }
        } trailing: {
            actions(for: provider)
        }
        .accessibilityElement(children: .combine)
    }

    private func detail(for provider: RouterProviderV1) -> String {
        switch provider.authKind {
        case "chatgpt_oauth":
            return "已通过官方登录 · 不需要 API Key"
        case "none":
            return "这家不需要凭据"
        case "bearer_key":
            if let key = provider.credentialKeyId, !key.isEmpty {
                return "已绑定 · 钥匙串条目 \(key)"
            }
            return "未绑定 —— 这家会被跳过"
        default:
            return provider.authKind
        }
    }

    @ViewBuilder
    private func actions(for provider: RouterProviderV1) -> some View {
        switch provider.authKind {
        case "bearer_key":
            if model.isMissingCredential(provider) {
                HStack(spacing: NativeSpacing.xs) {
                    NativeStatusBadge(.warning("未绑定"), theme: theme)
                    Button("绑定…") { onBind(provider) }
                        .controlSize(.small)
                }
            } else {
                HStack(spacing: NativeSpacing.xs) {
                    NativeStatusBadge(.ok("已绑定"), theme: theme)
                    // 只能替换,不能查看 —— 输入框是一次性的,关掉就再也读不回来。
                    Button("替换…") { onBind(provider) }
                        .controlSize(.small)
                    // 用系统的 destructive role,不要手工上色 —— 手工上色会渲成一个
                    // 实心色块,而系统 role 给的是 macOS 标准的破坏性按钮样式。
                    Button("解绑", role: .destructive) { onUnbind(provider) }
                        .controlSize(.small)
                }
            }
        case "chatgpt_oauth":
            NativeStatusBadge(.ok("已登录"), theme: theme)
        default:
            NativeStatusBadge(.inactive("不需要"), theme: theme)
        }
    }
}
#endif
