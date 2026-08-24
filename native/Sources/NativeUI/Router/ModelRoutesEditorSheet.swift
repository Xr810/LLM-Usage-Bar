#if os(macOS)
import SwiftUI
import UsageCore

/// 编辑一家 provider 的模型映射。写侧是**全量替换**,所以这里编辑的是整份清单。
public struct ModelRoutesEditorSheet: View {
    private let theme: NativeTheme
    private let provider: RouterProviderV1
    private let onCancel: () -> Void
    private let onSave: ([ModelRouteInputV1]) -> Void

    @State private var rows: [Row]
    @State private var pasteText = ""
    @State private var showsPaste = false

    private struct Row: Identifiable, Equatable {
        let id = UUID()
        var logical: String
        var upstream: String
    }

    public init(
        theme: NativeTheme,
        provider: RouterProviderV1,
        routes: [ModelRouteV1],
        onCancel: @escaping () -> Void,
        onSave: @escaping ([ModelRouteInputV1]) -> Void
    ) {
        self.theme = theme
        self.provider = provider
        self.onCancel = onCancel
        self.onSave = onSave
        _rows = State(
            initialValue: routes.map { Row(logical: $0.logicalModel, upstream: $0.upstreamModel) }
        )
    }

    private var cleanedRows: [ModelRouteInputV1] {
        var seen = Set<String>()
        return rows.compactMap { row in
            let logical = row.logical.trimmingCharacters(in: .whitespaces)
            guard !logical.isEmpty, seen.insert(logical).inserted else { return nil }
            let upstream = row.upstream.trimmingCharacters(in: .whitespaces)
            return ModelRouteInputV1(
                logicalModel: logical,
                upstreamModel: upstream.isEmpty ? logical : upstream
            )
        }
    }

    private var duplicateWarning: String? {
        let logicals = rows.map { $0.logical.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        return logicals.count == Set(logicals).count
            ? nil : "有重复的模型名,保存时只保留第一条"
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.md) {
            VStack(alignment: .leading, spacing: NativeSpacing.xxs) {
                Text("\(provider.displayName) 的模型映射")
                    .font(NativeTextStyle.sectionTitle)
                    .foregroundStyle(theme.textPrimary)
                Text("左边是 Codex 发来的模型名,右边是这家实际认的 ID。留空则按同名处理。")
                    .font(NativeTextStyle.label)
                    .foregroundStyle(theme.textSecondary)
            }

            NativeGroup(theme: theme) {
                header
                if rows.isEmpty {
                    NativeDivider(theme: theme, inset: 0)
                    Text("还没有映射。这家现在会被直接跳过。")
                        .font(NativeTextStyle.secondary)
                        .foregroundStyle(theme.textTertiary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(NativeSpacing.md)
                } else {
                    ForEach($rows) { $row in
                        NativeDivider(theme: theme, inset: 0)
                        editorRow($row)
                    }
                }
            }

            HStack(spacing: NativeSpacing.xs) {
                Button("添加一行") { rows.append(Row(logical: "", upstream: "")) }
                    .controlSize(.small)
                // 决定 11:手动粘贴是保底不是降级,所以它和「添加一行」并排,
                // 不是藏在角落的次要入口。
                Button(showsPaste ? "收起粘贴清单" : "粘贴清单…") { showsPaste.toggle() }
                    .controlSize(.small)
                Spacer()
                if let duplicateWarning {
                    NativeStatusBadge(.warning(duplicateWarning), theme: theme)
                }
            }

            if showsPaste { pasteBox }

            HStack {
                Text("\(cleanedRows.count) 条")
                    .font(NativeTextStyle.footnote)
                    .foregroundStyle(theme.textTertiary)
                Spacer()
                Button("取消", action: onCancel).keyboardShortcut(.cancelAction)
                Button("保存映射") { onSave(cleanedRows) }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(NativeSpacing.lg)
        .frame(width: 620)
        .background(theme.ground)
    }

    private var header: some View {
        HStack(spacing: NativeSpacing.sm) {
            Text("逻辑模型").frame(maxWidth: .infinity, alignment: .leading)
            Text("上游模型 ID").frame(maxWidth: .infinity, alignment: .leading)
            Color.clear.frame(width: 24)
        }
        .font(NativeTextStyle.label)
        .foregroundStyle(theme.textTertiary)
        .padding(.horizontal, NativeSpacing.md)
        .padding(.vertical, NativeSpacing.xs)
    }

    private func editorRow(_ row: Binding<Row>) -> some View {
        HStack(spacing: NativeSpacing.sm) {
            TextField("gpt-5.6", text: row.logical)
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: .infinity)
            TextField("与左边同名", text: row.upstream)
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: .infinity)
            Button {
                rows.removeAll { $0.id == row.wrappedValue.id }
            } label: {
                Image(systemName: "minus.circle")
            }
            .buttonStyle(.plain)
            .foregroundStyle(theme.textTertiary)
            .frame(width: 24)
            .accessibilityLabel("删除这一行")
        }
        .padding(.horizontal, NativeSpacing.md)
        .padding(.vertical, NativeSpacing.xs)
    }

    private var pasteBox: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.xs) {
            Text("每行一条。支持 `=`、`->`、`:`、逗号、制表符;只写一个名字就按同名处理。")
                .font(NativeTextStyle.footnote)
                .foregroundStyle(theme.textTertiary)
            TextEditor(text: $pasteText)
                .font(NativeTextStyle.tabularNumber)
                .frame(height: 120)
                .overlay(
                    NativeRadius.shape(NativeRadius.medium)
                        .strokeBorder(theme.hairline, lineWidth: 0.5)
                )
            HStack {
                let parsed = ModelRoutePaste.parse(pasteText)
                Text(parsed.isEmpty ? "还没解析出内容" : "解析出 \(parsed.count) 条")
                    .font(NativeTextStyle.footnote)
                    .foregroundStyle(theme.textTertiary)
                Spacer()
                Button("追加到上面") {
                    let existing = Set(rows.map { $0.logical.trimmingCharacters(in: .whitespaces) })
                    for route in parsed where !existing.contains(route.logicalModel) {
                        rows.append(
                            Row(logical: route.logicalModel, upstream: route.upstreamModel)
                        )
                    }
                    pasteText = ""
                    showsPaste = false
                }
                .controlSize(.small)
                .disabled(parsed.isEmpty)
                Button("替换全部") {
                    rows = parsed.map { Row(logical: $0.logicalModel, upstream: $0.upstreamModel) }
                    pasteText = ""
                    showsPaste = false
                }
                .controlSize(.small)
                .disabled(parsed.isEmpty)
            }
        }
    }
}
#endif

#if DEBUG
private let previewProvider = RouterProviderV1(
    id: "sol-relay", displayName: "Sol 中转",
    baseUrl: "https://api.sol-relay.dev/v1", wireApi: "responses",
    priority: 20, enabled: true, authKind: "bearer_key",
    credentialKeyId: "codex-router.sol"
)

#Preview("已有映射") {
    ModelRoutesEditorSheet(
        theme: NativeThemeCatalog.overcast,
        provider: previewProvider,
        routes: [
            ModelRouteV1(providerId: "sol-relay", logicalModel: "gpt-5.6",
                         upstreamModel: "sol-gpt-5.6-1120"),
            ModelRouteV1(providerId: "sol-relay", logicalModel: "gpt-5.6-sol",
                         upstreamModel: "sol-preview-1120"),
        ],
        onCancel: {},
        onSave: { _ in }
    )
}

#Preview("一条都没有") {
    ModelRoutesEditorSheet(
        theme: NativeThemeCatalog.overcast,
        provider: previewProvider,
        routes: [],
        onCancel: {},
        onSave: { _ in }
    )
}

#Preview("深色") {
    ModelRoutesEditorSheet(
        theme: NativeThemeCatalog.ink,
        provider: previewProvider,
        routes: [
            ModelRouteV1(providerId: "sol-relay", logicalModel: "gpt-5.6",
                         upstreamModel: "sol-gpt-5.6-1120"),
        ],
        onCancel: {},
        onSave: { _ in }
    )
    .environment(\.colorScheme, .dark)
}
#endif
