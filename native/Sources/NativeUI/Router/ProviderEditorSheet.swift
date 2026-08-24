#if os(macOS)
import SwiftUI
import UsageCore

/// 添加 / 编辑一家 provider。
///
/// **这里没有 priority。** 顺序归「模式与分账」那一节 —— 一件东西只能有一个编辑入口,
/// 否则两处都能改同一个值,用户永远不知道哪个说了算。
public struct ProviderEditorSheet: View {
    private let theme: NativeTheme
    private let existing: RouterProviderV1?
    private let takenIds: Set<String>
    private let onCancel: () -> Void
    private let onSave: (RouterProviderV1) -> Void

    @State private var id: String
    @State private var displayName: String
    @State private var baseUrl: String
    @State private var wireApi: String
    @State private var authKind: String
    @State private var enabled: Bool

    public init(
        theme: NativeTheme,
        existing: RouterProviderV1?,
        takenIds: Set<String>,
        onCancel: @escaping () -> Void,
        onSave: @escaping (RouterProviderV1) -> Void
    ) {
        self.theme = theme
        self.existing = existing
        self.takenIds = takenIds
        self.onCancel = onCancel
        self.onSave = onSave
        _id = State(initialValue: existing?.id ?? "")
        _displayName = State(initialValue: existing?.displayName ?? "")
        _baseUrl = State(initialValue: existing?.baseUrl ?? "")
        _wireApi = State(initialValue: existing?.wireApi ?? "responses")
        _authKind = State(initialValue: existing?.authKind ?? "bearer_key")
        _enabled = State(initialValue: existing?.enabled ?? true)
    }

    private var isEditing: Bool { existing != nil }

    /// id 是主键,建好之后不给改 —— 改 id 等于换一家,映射和分账都会跟着断。
    private var idIsLocked: Bool { isEditing }

    private var trimmedId: String { id.trimmingCharacters(in: .whitespaces) }
    private var trimmedUrl: String { baseUrl.trimmingCharacters(in: .whitespaces) }

    private var validationError: String? {
        if trimmedId.isEmpty { return "标识不能为空" }
        if !idIsLocked && takenIds.contains(trimmedId) { return "已经有一家用了这个标识" }
        if trimmedId.contains(where: { $0.isWhitespace }) { return "标识里不能有空格" }
        if trimmedUrl.isEmpty { return "Base URL 不能为空" }
        if !(trimmedUrl.hasPrefix("http://") || trimmedUrl.hasPrefix("https://")) {
            return "Base URL 要以 http:// 或 https:// 开头"
        }
        return nil
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.md) {
            Text(isEditing ? "编辑 Provider" : "添加 Provider")
                .font(NativeTextStyle.sectionTitle)
                .foregroundStyle(theme.textPrimary)

            NativeGroup(theme: theme) {
                field("标识") {
                    TextField("sol-relay", text: $id)
                        .disabled(idIsLocked)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 260)
                }
                NativeDivider(theme: theme)
                field("显示名") {
                    TextField("Sol 中转", text: $displayName)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 260)
                }
                NativeDivider(theme: theme)
                field("Base URL") {
                    TextField("https://api.example.com/v1", text: $baseUrl)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 260)
                }
                NativeDivider(theme: theme)
                field("协议") {
                    Picker("", selection: $wireApi) {
                        Text("Responses").tag("responses")
                        Text("Chat Completions").tag("chat_completions")
                    }
                    .labelsHidden()
                    .frame(width: 260)
                }
                NativeDivider(theme: theme)
                field("凭据方式") {
                    Picker("", selection: $authKind) {
                        Text("官方登录(OAuth)").tag("chatgpt_oauth")
                        Text("API Key").tag("bearer_key")
                        Text("不需要凭据").tag("none")
                    }
                    .labelsHidden()
                    .frame(width: 260)
                }
                NativeDivider(theme: theme)
                field("启用") {
                    Toggle("", isOn: $enabled).labelsHidden()
                }
            }

            if idIsLocked {
                Text("标识建好之后不能改 —— 改它等于换一家,已有的映射和分账会跟着断。")
                    .font(NativeTextStyle.footnote)
                    .foregroundStyle(theme.textTertiary)
            }

            if authKind == "bearer_key" && existing?.credentialKeyId == nil {
                Text("保存之后记得在「凭据」那一节绑定 API Key,否则这家会被跳过。")
                    .font(NativeTextStyle.footnote)
                    .foregroundStyle(theme.textSecondary)
            }

            HStack(spacing: NativeSpacing.xs) {
                if let validationError {
                    NativeStatusBadge(.warning(validationError), theme: theme)
                }
                Spacer()
                Button("取消", action: onCancel)
                    .keyboardShortcut(.cancelAction)
                Button(isEditing ? "保存" : "添加") {
                    onSave(
                        RouterProviderV1(
                            id: trimmedId,
                            displayName: displayName.trimmingCharacters(in: .whitespaces)
                                .isEmpty ? trimmedId : displayName,
                            baseUrl: trimmedUrl,
                            wireApi: wireApi,
                            // 新建的排在最后,顺序让用户去「模式与分账」里调。
                            priority: existing?.priority ?? 999,
                            enabled: enabled,
                            authKind: authKind,
                            credentialKeyId: existing?.credentialKeyId
                        )
                    )
                }
                .keyboardShortcut(.defaultAction)
                .disabled(validationError != nil)
            }
        }
        .padding(NativeSpacing.lg)
        .frame(width: 520)
        .background(theme.ground)
    }

    private func field<Control: View>(
        _ label: String,
        @ViewBuilder control: () -> Control
    ) -> some View {
        NativeRow(theme: theme) {
            Text(label)
                .font(NativeTextStyle.body)
                .foregroundStyle(theme.textPrimary)
        } trailing: {
            control()
        }
    }
}
#endif

#if DEBUG
// ImageRenderer 渲不了 TextField / Picker / Toggle(会出黄色占位块),
// 所以这两个 sheet 只能在 Xcode canvas 或真窗口里验。
#Preview("添加 Provider") {
    ProviderEditorSheet(
        theme: NativeThemeCatalog.overcast,
        existing: nil,
        takenIds: ["official"],
        onCancel: {},
        onSave: { _ in }
    )
}

#Preview("编辑 Provider · 标识锁定") {
    ProviderEditorSheet(
        theme: NativeThemeCatalog.overcast,
        existing: RouterProviderV1(
            id: "sol-relay", displayName: "Sol 中转",
            baseUrl: "https://api.sol-relay.dev/v1", wireApi: "responses",
            priority: 20, enabled: true, authKind: "bearer_key",
            credentialKeyId: "codex-router.sol"
        ),
        takenIds: ["official", "sol-relay"],
        onCancel: {},
        onSave: { _ in }
    )
}

#Preview("编辑 Provider · 深色") {
    ProviderEditorSheet(
        theme: NativeThemeCatalog.ink,
        existing: nil,
        takenIds: [],
        onCancel: {},
        onSave: { _ in }
    )
    .environment(\.colorScheme, .dark)
}
#endif
