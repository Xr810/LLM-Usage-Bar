#if os(macOS)
import SwiftUI

// 分组表单的基本件。取自 Raycast 设置窗的读法(2026-08-23 视觉方向 §4):
// 标签在左、控件在右、组内细分隔线、分区标题上方留大间距。
//
// 关键的一条:**透气来自文字对比与元数据右对齐,不是把行撑高。**
// 实测 Raycast 行高约 38pt,比"抬高行高"的方案还紧。

/// 分区标题。上方留大间距是它和普通文字的主要区别。
public struct NativeSectionHeader: View {
    private let title: String
    private let subtitle: String?
    private let trailing: AnyView?
    private let theme: NativeTheme

    public init<Trailing: View>(
        _ title: String,
        subtitle: String? = nil,
        theme: NativeTheme,
        @ViewBuilder trailing: () -> Trailing
    ) {
        self.title = title
        self.subtitle = subtitle
        self.theme = theme
        self.trailing = AnyView(trailing())
    }

    public init(_ title: String, subtitle: String? = nil, theme: NativeTheme) {
        self.title = title
        self.subtitle = subtitle
        self.theme = theme
        self.trailing = nil
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: NativeSpacing.xxs) {
            HStack(alignment: .firstTextBaseline, spacing: NativeSpacing.xs) {
                Text(title)
                    .font(NativeTextStyle.sectionTitle)
                    .foregroundStyle(theme.textPrimary)
                Spacer(minLength: NativeSpacing.xs)
                trailing
            }
            if let subtitle {
                Text(subtitle)
                    .font(NativeTextStyle.label)
                    .foregroundStyle(theme.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, NativeSpacing.xxs)
    }
}

/// 一组行。圆角面板 + 组内细分隔线,不是每行一张带阴影的卡片。
public struct NativeGroup<Content: View>: View {
    private let theme: NativeTheme
    private let content: Content

    public init(theme: NativeTheme, @ViewBuilder content: () -> Content) {
        self.theme = theme
        self.content = content()
    }

    public var body: some View {
        VStack(spacing: 0) { content }
            .background(theme.group)
            .clipShape(NativeRadius.shape(NativeRadius.large))
            .overlay(
                NativeRadius.shape(NativeRadius.large)
                    .strokeBorder(theme.hairline, lineWidth: 0.5)
            )
    }
}

/// 组内分隔线。左侧内缩,与 macOS 的 grouped form 一致。
public struct NativeDivider: View {
    private let theme: NativeTheme
    private let inset: CGFloat

    public init(theme: NativeTheme, inset: CGFloat = NativeSpacing.md) {
        self.theme = theme
        self.inset = inset
    }

    public var body: some View {
        Rectangle()
            .fill(theme.hairline)
            .frame(height: 0.5)
            .padding(.leading, inset)
    }
}

/// provider 身份色点。小到只是一个标记,不构成视觉重量。
///
/// **不能只靠颜色传达信息**(系统有「不使用颜色传达信息」开关),所以它永远和
/// provider 名字并排出现 —— 色点是加速识别的,名字才是信息本身。
public struct NativeIdentityDot: View {
    private let index: Int
    private let appearance: NativeAppearance

    public init(index: Int, appearance: NativeAppearance) {
        self.index = index
        self.appearance = appearance
    }

    public var body: some View {
        Circle()
            .fill(NativeIdentityPalette.color(forIndex: index).resolved(for: appearance))
            .frame(width: NativeIdentityPalette.dotSize, height: NativeIdentityPalette.dotSize)
            .accessibilityHidden(true)
    }
}

/// 状态徽标。颜色只上在符号上,不上在行背景。
public struct NativeStatusBadge: View {
    public enum Kind: Equatable, Sendable {
        case ok(String)
        case warning(String)
        /// 已停用 / 不参与 —— 无色
        case inactive(String)
        /// 暂不可用倒计时 —— **临时状态不是错误**,同样无色
        case cooldown(String)
    }

    private let kind: Kind
    private let theme: NativeTheme

    public init(_ kind: Kind, theme: NativeTheme) {
        self.kind = kind
        self.theme = theme
    }

    private var symbol: String {
        switch kind {
        case .ok: return NativeStatusColor.Symbol.ok
        case .warning: return NativeStatusColor.Symbol.warning
        case .inactive: return NativeStatusColor.Symbol.inactive
        case .cooldown: return NativeStatusColor.Symbol.cooldown
        }
    }

    private var tint: Color {
        switch kind {
        case .ok: return NativeStatusColor.ok.resolved(for: theme.appearance)
        case .warning: return NativeStatusColor.warning.resolved(for: theme.appearance)
        case .inactive, .cooldown: return theme.textTertiary
        }
    }

    private var label: String {
        switch kind {
        case let .ok(text), let .warning(text), let .inactive(text), let .cooldown(text):
            return text
        }
    }

    public var body: some View {
        HStack(spacing: NativeSpacing.xxs) {
            Image(systemName: symbol)
                .foregroundStyle(tint)
            Text(label)
                .foregroundStyle(theme.textSecondary)
        }
        .font(NativeTextStyle.label)
        // 形状(SF Symbol)与文字都在,颜色只是第三重编码 —— 关掉颜色仍然可读。
        .accessibilityElement(children: .combine)
        .accessibilityLabel(label)
    }
}

/// 一行:标签在左(可带压暗的第二行),控件/状态在右。
public struct NativeRow<Leading: View, Trailing: View>: View {
    private let theme: NativeTheme
    private let leading: Leading
    private let trailing: Trailing

    public init(
        theme: NativeTheme,
        @ViewBuilder leading: () -> Leading,
        @ViewBuilder trailing: () -> Trailing
    ) {
        self.theme = theme
        self.leading = leading()
        self.trailing = trailing()
    }

    public var body: some View {
        HStack(alignment: .center, spacing: NativeSpacing.sm) {
            leading
            Spacer(minLength: NativeSpacing.sm)
            trailing
        }
        .padding(.horizontal, NativeSpacing.md)
        .padding(.vertical, NativeSpacing.sm)
    }
}

/// 主标签 + 压暗的次要说明。两级对比拉开是这套观感的核心。
public struct NativeRowLabel: View {
    private let title: String
    private let detail: String?
    private let theme: NativeTheme
    private let monospacedDetail: Bool

    public init(
        _ title: String,
        detail: String? = nil,
        theme: NativeTheme,
        monospacedDetail: Bool = false
    ) {
        self.title = title
        self.detail = detail
        self.theme = theme
        self.monospacedDetail = monospacedDetail
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(NativeTextStyle.body)
                .foregroundStyle(theme.textPrimary)
            if let detail {
                Text(detail)
                    .font(monospacedDetail ? NativeTextStyle.tabularNumber : NativeTextStyle.label)
                    .foregroundStyle(theme.textSecondary)
            }
        }
    }
}

/// 钉在顶部的错误条。**不自动消失** —— 设计要求用户必须看到并处理。
public struct NativeErrorBanner: View {
    private let message: String
    private let theme: NativeTheme
    private let onDismiss: () -> Void

    public init(message: String, theme: NativeTheme, onDismiss: @escaping () -> Void) {
        self.message = message
        self.theme = theme
        self.onDismiss = onDismiss
    }

    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: NativeSpacing.xs) {
            Image(systemName: NativeStatusColor.Symbol.warning)
                .foregroundStyle(NativeStatusColor.warning.resolved(for: theme.appearance))
            Text(message)
                .font(NativeTextStyle.secondary)
                .foregroundStyle(theme.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: NativeSpacing.xs)
            Button("知道了", action: onDismiss)
                .buttonStyle(.plain)
                .font(NativeTextStyle.label)
                .foregroundStyle(theme.accent)
        }
        .padding(.horizontal, NativeSpacing.md)
        .padding(.vertical, NativeSpacing.sm)
        .background(NativeStatusColor.warning.resolved(for: theme.appearance).opacity(0.12))
        .clipShape(NativeRadius.shape(NativeRadius.medium))
        .accessibilityElement(children: .combine)
    }
}
#endif
