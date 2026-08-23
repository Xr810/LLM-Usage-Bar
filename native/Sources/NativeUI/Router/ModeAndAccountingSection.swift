#if os(macOS)
import SwiftUI
import UsageCore

/// 第四节:模式与分账。
///
/// 模式切换器**换的是下面那块区域本身**,不是给同一张列表换文案 ——
/// 两个模式下你要操作的东西根本不是同一件:自动模式要调的是**顺序**,
/// 手动模式要点的是**哪一家**。
public struct ModeAndAccountingSection: View {
    @ObservedObject private var model: RouterPanelModel
    private let theme: NativeTheme
    private let onModeChange: (RouterModeV1) -> Void
    private let onMove: (IndexSet, Int) -> Void

    public init(
        model: RouterPanelModel,
        theme: NativeTheme,
        onModeChange: @escaping (RouterModeV1) -> Void,
        onMove: @escaping (IndexSet, Int) -> Void
    ) {
        self.model = model
        self.theme = theme
        self.onModeChange = onModeChange
        self.onMove = onMove
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.sm) {
            NativeSectionHeader("模式与分账", theme: theme)

            // 决定 31:手动模式必须**持续可见**,不是切换时提示一下。
            // 切了忘了、某家挂了没自动换,会被误判成「故障转移坏了」。
            if let manualId = model.manuallySelectedProviderId {
                manualBanner(providerId: manualId)
            }

            NativeGroup(theme: theme) {
                modePicker
                NativeDivider(theme: theme, inset: 0)
                if model.manuallySelectedProviderId == nil {
                    orderingList
                } else {
                    pickingList
                }
            }

            accounting
        }
    }

    // MARK: 模式

    private var modePicker: some View {
        NativeRow(theme: theme) {
            Text("模式")
                .font(NativeTextStyle.body)
                .foregroundStyle(theme.textPrimary)
        } trailing: {
            HStack(spacing: NativeSpacing.sm) {
                Picker("模式", selection: modeBinding) {
                    Text("自动").tag(false)
                    Text("手动").tag(true)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()

                Text(model.manuallySelectedProviderId == nil
                    ? "按顺序尝试,失败自动换下一家"
                    : "只用你选的那一家,系统不会自己切回自动")
                    .font(NativeTextStyle.label)
                    .foregroundStyle(theme.textSecondary)
            }
        }
    }

    private var modeBinding: Binding<Bool> {
        Binding(
            get: { model.manuallySelectedProviderId != nil },
            set: { isManual in
                if isManual {
                    guard let first = model.providersInTryOrder.first else { return }
                    onModeChange(.manual(providerId: first.id))
                } else {
                    onModeChange(.auto)
                }
            }
        )
    }

    private func manualBanner(providerId: String) -> some View {
        let name = model.providers.first { $0.id == providerId }?.displayName ?? providerId
        return HStack(spacing: NativeSpacing.xs) {
            Image(systemName: NativeStatusColor.Symbol.warning)
                .foregroundStyle(NativeStatusColor.warning.resolved(for: theme.appearance))
            Text("手动模式 · 只用 \(name)。不会自动切换,这家失败就是失败。")
                .font(NativeTextStyle.label)
                .foregroundStyle(theme.textPrimary)
            Spacer(minLength: NativeSpacing.xs)
            Button("切回自动") { onModeChange(.auto) }
                .controlSize(.small)
        }
        .padding(NativeSpacing.sm)
        .background(NativeStatusColor.warning.resolved(for: theme.appearance).opacity(0.12))
        .clipShape(NativeRadius.shape(NativeRadius.medium))
        .accessibilityElement(children: .combine)
    }

    // MARK: 自动 —— 排序

    /// 自动模式下唯一有意义的输入就是顺序:先试谁、失败换谁。
    private var orderingList: some View {
        VStack(alignment: .leading, spacing: 0) {
            NativeRow(theme: theme) {
                Text("尝试顺序")
                    .font(NativeTextStyle.label)
                    .foregroundStyle(theme.textSecondary)
            } trailing: {
                Text("从上往下依次尝试")
                    .font(NativeTextStyle.footnote)
                    .foregroundStyle(theme.textTertiary)
            }

            List {
                ForEach(Array(model.providersInTryOrder.enumerated()), id: \.element.id) {
                    index, provider in
                    orderingRow(provider, index: index)
                        .listRowInsets(EdgeInsets())
                        .listRowSeparator(.hidden)
                        .listRowBackground(Color.clear)
                }
                .onMove(perform: onMove)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .scrollDisabled(true)
            .frame(height: CGFloat(max(model.providers.count, 1)) * 44)

            Text("拖右侧把手,或选中一行后按 ⌥⌘↑ / ⌥⌘↓ 移动。")
                .font(NativeTextStyle.footnote)
                .foregroundStyle(theme.textTertiary)
                .padding(.horizontal, NativeSpacing.md)
                .padding(.bottom, NativeSpacing.sm)
        }
    }

    private func orderingRow(_ provider: RouterProviderV1, index: Int) -> some View {
        NativeRow(theme: theme) {
            HStack(spacing: NativeSpacing.xs) {
                Text("\(index + 1)")
                    .font(NativeTextStyle.footnote)
                    .foregroundStyle(theme.textTertiary)
                    .frame(width: 16, alignment: .trailing)
                NativeIdentityDot(index: index, appearance: theme.appearance)
                NativeRowLabel(
                    provider.displayName,
                    detail: model.isMissingCredential(provider) ? "缺少凭据 · 会被跳过" : nil,
                    theme: theme
                )
            }
        } trailing: {
            HStack(spacing: NativeSpacing.sm) {
                if !provider.enabled {
                    NativeStatusBadge(.inactive("已停用"), theme: theme)
                } else if model.isMissingCredential(provider) {
                    NativeStatusBadge(.warning("缺少凭据"), theme: theme)
                }
                Image(systemName: "line.3.horizontal")
                    .foregroundStyle(theme.textTertiary)
                    .accessibilityHidden(true)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            "第 \(index + 1) 项,共 \(model.providers.count) 项,\(provider.displayName)"
        )
    }

    // MARK: 手动 —— 点选

    /// 手动模式下要点的是「哪一家」。这张列表和菜单栏 popover 里那张是同一个控件的
    /// 两个尺寸,只差行高与副标题密度。
    private var pickingList: some View {
        VStack(alignment: .leading, spacing: 0) {
            NativeRow(theme: theme) {
                Text("使用哪一家")
                    .font(NativeTextStyle.label)
                    .foregroundStyle(theme.textSecondary)
            } trailing: {
                Text("点一行即切换")
                    .font(NativeTextStyle.footnote)
                    .foregroundStyle(theme.textTertiary)
            }

            ForEach(Array(model.providersInTryOrder.enumerated()), id: \.element.id) {
                index, provider in
                NativeDivider(theme: theme)
                pickingRow(provider, index: index)
            }

            Text("被标为不可用的那家在手动模式下**照样可以选** —— 拉黑只是提示,不生效。")
                .font(NativeTextStyle.footnote)
                .foregroundStyle(theme.textTertiary)
                .padding(.horizontal, NativeSpacing.md)
                .padding(.vertical, NativeSpacing.sm)
        }
    }

    private func pickingRow(_ provider: RouterProviderV1, index: Int) -> some View {
        let isSelected = model.manuallySelectedProviderId == provider.id
        // 缺凭据是真的不能用;被拉黑只是提示,仍然可选(决定 29)。
        let selectable = !model.isMissingCredential(provider)

        return Button {
            onModeChange(.manual(providerId: provider.id))
        } label: {
            NativeRow(theme: theme) {
                HStack(spacing: NativeSpacing.xs) {
                    Image(systemName: NativeStatusColor.Symbol.ok)
                        .foregroundStyle(
                            isSelected
                                ? NativeStatusColor.ok.resolved(for: theme.appearance)
                                : Color.clear
                        )
                    NativeIdentityDot(index: index, appearance: theme.appearance)
                    NativeRowLabel(provider.displayName, detail: provider.baseUrl, theme: theme)
                }
            } trailing: {
                if isSelected {
                    Text("正在使用")
                        .font(NativeTextStyle.label)
                        .foregroundStyle(theme.textSecondary)
                } else if model.isMissingCredential(provider) {
                    NativeStatusBadge(.warning("缺少凭据"), theme: theme)
                }
            }
        }
        .buttonStyle(.plain)
        .disabled(!selectable)
        .background(isSelected ? theme.selection : Color.clear)
        .accessibilityAddTraits(isSelected ? [.isSelected] : [])
    }

    // MARK: 分账

    private var accounting: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.xs) {
            NativeSectionHeader("分账", theme: theme)

            // 这段不能省。这些数字**是下界,不是账单**。
            Text("只统计经过路由、并且拿到了 usage 的请求。中途取消的、上游断流的、"
                + "上游不报 usage 的都不在内 —— 实际用量只会比这里多。")
                .font(NativeTextStyle.label)
                .foregroundStyle(theme.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, NativeSpacing.xxs)

            NativeGroup(theme: theme) {
                accountingHeader
                if model.attempts.isEmpty {
                    NativeDivider(theme: theme, inset: 0)
                    emptyAccounting
                } else {
                    ForEach(model.attempts, id: \.providerId) { row in
                        NativeDivider(theme: theme, inset: 0)
                        accountingRow(row)
                    }
                    NativeDivider(theme: theme, inset: 0)
                    accountingTotals
                }
            }
        }
    }

    private var accountingHeader: some View {
        HStack(spacing: NativeSpacing.sm) {
            Text("Provider").frame(maxWidth: .infinity, alignment: .leading)
            Text("尝试").frame(width: 56, alignment: .trailing)
            Text("失败").frame(width: 56, alignment: .trailing)
            // 「≥」不是装饰:它是「下界」这件事在表头上的落点。
            Text("输入 ≥").frame(width: 96, alignment: .trailing)
            Text("输出 ≥").frame(width: 96, alignment: .trailing)
        }
        .font(NativeTextStyle.label)
        .foregroundStyle(theme.textTertiary)
        .padding(.horizontal, NativeSpacing.md)
        .padding(.vertical, NativeSpacing.xs)
    }

    private func accountingRow(_ row: RouterUsageSummaryV1) -> some View {
        let provider = model.providers.first { $0.id == row.providerId }
        return HStack(spacing: NativeSpacing.sm) {
            HStack(spacing: NativeSpacing.xs) {
                NativeIdentityDot(
                    index: model.identityIndex(forProvider: row.providerId),
                    appearance: theme.appearance
                )
                Text(provider?.displayName ?? row.providerId)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Text("\(row.attempts)").frame(width: 56, alignment: .trailing)
            Text("\(row.failures)").frame(width: 56, alignment: .trailing)
            Text(Self.grouped(row.inputTokens)).frame(width: 96, alignment: .trailing)
            Text(Self.grouped(row.outputTokens)).frame(width: 96, alignment: .trailing)
        }
        .font(NativeTextStyle.tabularNumber)
        .foregroundStyle(theme.textPrimary)
        .padding(.horizontal, NativeSpacing.md)
        .padding(.vertical, NativeSpacing.xs)
        .accessibilityElement(children: .combine)
    }

    private var accountingTotals: some View {
        let totals = model.accountingTotals
        return HStack(spacing: NativeSpacing.sm) {
            Text("合计").frame(maxWidth: .infinity, alignment: .leading)
            Text("\(totals.attempts)").frame(width: 56, alignment: .trailing)
            Text("\(totals.failures)").frame(width: 56, alignment: .trailing)
            Text(Self.grouped(totals.inputTokens)).frame(width: 96, alignment: .trailing)
            Text(Self.grouped(totals.outputTokens)).frame(width: 96, alignment: .trailing)
        }
        .font(NativeTextStyle.tabularNumber.weight(.semibold))
        .foregroundStyle(theme.textPrimary)
        .padding(.horizontal, NativeSpacing.md)
        .padding(.vertical, NativeSpacing.xs)
    }

    private var emptyAccounting: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.xxs) {
            Text("这段时间还没有经过路由的请求")
                .font(NativeTextStyle.body)
                .foregroundStyle(theme.textPrimary)
            // 空态把重启这件事接上 —— 这是用户最容易踩的坑。
            Text("Codex 重启之后才会走这里。已接管但没重启的话,请求仍然直连原来那家,"
                + "也就不会被记账。")
                .font(NativeTextStyle.label)
                .foregroundStyle(theme.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(NativeSpacing.md)
    }

    private static func grouped(_ value: Int64) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        return formatter.string(from: NSNumber(value: value)) ?? "\(value)"
    }
}
#endif
