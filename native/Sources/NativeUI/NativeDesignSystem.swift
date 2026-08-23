#if os(macOS)
import SwiftUI

// 设计系统的**唯一**颜色定义处。
//
// 规矩(HIG 改造计划 P0 的 lint 三条 + 2026-08-23 视觉方向):
//   1. 字面颜色只允许出现在本文件里,别处一律走 token
//   2. 字号只用语义刻度,禁止 .font(.system(size:))
//   3. 圆角只用 NativeRadius 的四档,且全部 .continuous
//
// 颜色出现在且只出现在三处:导航(选中态)、身份(provider 色点)、状态(符号)。
// 数据区(映射表、分账表正文)不取色 —— 一上色就变成网页仪表盘。

// MARK: - 间距与圆角

/// 8pt 网格 + 半档。旧代码散落着 3/4/6/9/10/13/14/15/16/20/22,收敛到这六个。
public enum NativeSpacing {
    public static let xxs: CGFloat = 4
    public static let xs: CGFloat = 8
    public static let sm: CGFloat = 12
    public static let md: CGFloat = 16
    public static let lg: CGFloat = 20
    public static let xl: CGFloat = 24
}

/// 四档,全部 `.continuous` —— 普通圆弧一眼看出是网页。
public enum NativeRadius {
    /// 角标、小按钮
    public static let small: CGFloat = 6
    /// 行、输入框
    public static let medium: CGFloat = 10
    /// 卡片、分组面板
    public static let large: CGFloat = 14
    /// 弹出层、窗口
    public static let window: CGFloat = 16

    public static func shape(_ radius: CGFloat) -> RoundedRectangle {
        RoundedRectangle(cornerRadius: radius, style: .continuous)
    }
}

// MARK: - 字号

/// macOS 的语义刻度。**没有 iOS 那样的 Dynamic Type**,所以用它们的理由不是自动缩放,
/// 而是:这就是苹果自己那套刻度,用了才能和别的 Mac app 的层次对齐。
public enum NativeTextStyle {
    /// 22pt —— 主窗口大数字
    public static let displayNumber: Font = .title.monospacedDigit()
    /// 17pt —— 卡片主数值
    public static let cardNumber: Font = .title2.monospacedDigit()
    /// 13pt semibold —— 区块标题
    public static let sectionTitle: Font = .headline
    /// 13pt —— 正文
    public static let body: Font = .body
    /// 12pt —— 次级正文
    public static let secondary: Font = .callout
    /// 11pt —— 行内标签
    public static let label: Font = .subheadline
    /// 10pt —— 辅助说明。**下限就是这里**,9pt 低于苹果任何一档、Retina 上也偏糊
    public static let footnote: Font = .footnote
    /// 表格里的数字:等宽数字,列才对得齐
    public static let tabularNumber: Font = .body.monospacedDigit()
}

// MARK: - 颜色基元

/// 一个颜色在浅/深两套外观下的取值。身份色不是一个常量 —— 深色下要换变体,
/// 否则在深底上发闷。
public struct NativeColorPair: Sendable, Equatable {
    public let light: Color
    public let dark: Color

    public init(light: Color, dark: Color) {
        self.light = light
        self.dark = dark
    }

    public init(both value: Color) {
        self.light = value
        self.dark = value
    }

    public func resolved(for appearance: NativeAppearance) -> Color {
        appearance == .dark ? dark : light
    }
}

extension Color {
    /// 只在本文件内使用 —— 别处出现字面颜色就是违规。
    fileprivate init(hex: UInt32, opacity: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: opacity
        )
    }
}

public enum NativeAppearance: String, Sendable, CaseIterable {
    case light
    case dark
}

// MARK: - 状态色

/// 状态色**只上在符号上,不上在行背景**。取值是 macOS 系统色的真实值。
///
/// 无障碍:系统有「不使用颜色传达信息」开关,所以每个状态都必须**同时**有形状区分 ——
/// 这里给出配套的 SF Symbol 名,调用方不要只用颜色。
public enum NativeStatusColor {
    /// 已登录 / 凭据已绑定 / 路由运行中
    public static let ok = NativeColorPair(light: Color(hex: 0x28CD41), dark: Color(hex: 0x32D74B))
    /// 缺凭据 / 待绑定
    public static let warning =
        NativeColorPair(light: Color(hex: 0xFFCC00), dark: Color(hex: 0xFFD60A))
    /// 破坏性操作(解绑、删除映射)。**不用于任何 provider 身份**
    public static let destructive =
        NativeColorPair(light: Color(hex: 0xFF3B30), dark: Color(hex: 0xFF453A))

    public enum Symbol {
        public static let ok = "checkmark.circle.fill"
        public static let warning = "exclamationmark.triangle.fill"
        /// 已停用 / 不参与 —— 二级灰,不给颜色
        public static let inactive = "minus.circle"
        /// 暂不可用倒计时 —— **临时状态不是错误**,同样不给颜色
        public static let cooldown = "clock"
        public static let running = "circle.fill"
    }

    /// 黄底太亮,笔画取深色而不是白 —— 这是苹果自己的画法。
    public static let warningGlyph = Color(hex: 0x000000, opacity: 0.72)
}

// MARK: - 身份色

/// provider 前导色点。按注册顺序分配、可在设置里改,第 7 家起回到头循环。
///
/// **硬规则:身份色不能和状态色撞。** green / yellow / red 保留给状态,
/// 纯蓝避开(那是默认强调色),所以这六个是 indigo / orange / purple / teal / pink / brown。
public enum NativeIdentityPalette {
    public static let colors: [NativeColorPair] = [
        NativeColorPair(light: Color(hex: 0x5856D6), dark: Color(hex: 0x5E5CE6)),  // indigo
        NativeColorPair(light: Color(hex: 0xFF9500), dark: Color(hex: 0xFF9F0A)),  // orange
        NativeColorPair(light: Color(hex: 0xAF52DE), dark: Color(hex: 0xBF5AF2)),  // purple
        NativeColorPair(light: Color(hex: 0x59ADC4), dark: Color(hex: 0x6AC4DC)),  // teal
        NativeColorPair(light: Color(hex: 0xFF2D55), dark: Color(hex: 0xFF375F)),  // pink
        NativeColorPair(light: Color(hex: 0xA2845E), dark: Color(hex: 0xAC8E68)),  // brown
    ]

    /// 色点直径。小到只是一个身份标记,不构成视觉重量。
    public static let dotSize: CGFloat = 8

    public static func color(forIndex index: Int) -> NativeColorPair {
        colors[((index % colors.count) + colors.count) % colors.count]
    }
}

// MARK: - 主题

/// 一套主题定义的是**底色、面、三级文字、分隔、选中态与强调色**。
/// 不包含状态色与身份色 —— 那两组跨主题保持一致,否则「绿=好」这类语义会随主题漂移。
public struct NativeTheme: Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let appearance: NativeAppearance

    /// 关掉半透明时的窗口底色。**不是白色** —— 纯白是「没设计过」的样子。
    public let ground: Color
    /// 工具栏 / 底部操作条
    public let toolbar: Color
    /// 分组面板
    public let group: Color
    /// 组内分隔线
    public let hairline: Color
    /// 选中态。**极淡的半透明,不是强调色的带子** —— 它该读起来是「被照亮」,不是「被涂色」
    public let selection: Color

    public let textPrimary: Color
    public let textSecondary: Color
    public let textTertiary: Color

    public let controlFill: Color
    public let controlBorder: Color

    /// `nil` 表示**跟随用户在系统设置里选的强调色**(内置的「系统」主题就是这样)。
    /// 见视觉方向 V3:强调色由主题定,但默认那套跟随系统。
    public let accentOverride: Color?

    public var accent: Color { accentOverride ?? .accentColor }

    public init(
        id: String,
        name: String,
        appearance: NativeAppearance,
        ground: Color,
        toolbar: Color,
        group: Color,
        hairline: Color,
        selection: Color,
        textPrimary: Color,
        textSecondary: Color,
        textTertiary: Color,
        controlFill: Color,
        controlBorder: Color,
        accentOverride: Color?
    ) {
        self.id = id
        self.name = name
        self.appearance = appearance
        self.ground = ground
        self.toolbar = toolbar
        self.group = group
        self.hairline = hairline
        self.selection = selection
        self.textPrimary = textPrimary
        self.textSecondary = textSecondary
        self.textTertiary = textTertiary
        self.controlFill = controlFill
        self.controlBorder = controlBorder
        self.accentOverride = accentOverride
    }
}

/// 起步两浅一深里的「两浅」其实是一浅一「系统」:
/// `Sage` 与 `Overcast` 并排看几乎分不出,两套差别不明显的主题不如一套明确的(V5)。
public enum NativeThemeCatalog {
    /// 跟随系统:中性灰用 AppKit 的语义色,强调色跟随用户设置。**默认就是它。**
    public static let systemLight = NativeTheme(
        id: "system",
        name: "系统",
        appearance: .light,
        ground: Color(nsColor: .windowBackgroundColor),
        toolbar: Color(nsColor: .underPageBackgroundColor),
        group: Color(nsColor: .controlBackgroundColor),
        hairline: Color(nsColor: .separatorColor),
        selection: Color(nsColor: .unemphasizedSelectedContentBackgroundColor),
        textPrimary: Color(nsColor: .labelColor),
        textSecondary: Color(nsColor: .secondaryLabelColor),
        textTertiary: Color(nsColor: .tertiaryLabelColor),
        controlFill: Color(nsColor: .controlColor),
        controlBorder: Color(nsColor: .separatorColor),
        accentOverride: nil
    )

    public static let systemDark = NativeTheme(
        id: "system",
        name: "系统",
        appearance: .dark,
        ground: Color(nsColor: .windowBackgroundColor),
        toolbar: Color(nsColor: .underPageBackgroundColor),
        group: Color(nsColor: .controlBackgroundColor),
        hairline: Color(nsColor: .separatorColor),
        selection: Color(nsColor: .unemphasizedSelectedContentBackgroundColor),
        textPrimary: Color(nsColor: .labelColor),
        textSecondary: Color(nsColor: .secondaryLabelColor),
        textTertiary: Color(nsColor: .tertiaryLabelColor),
        controlFill: Color(nsColor: .controlColor),
        controlBorder: Color(nsColor: .separatorColor),
        accentOverride: nil
    )

    /// 冷灰偏蓝紫。带一点色偏正是它跟「纯中性灰」的区别 —— 纯灰读起来像没设计过。
    public static let overcast = NativeTheme(
        id: "overcast",
        name: "Overcast",
        appearance: .light,
        ground: Color(hex: 0xECEEF4),
        toolbar: Color(hex: 0xFFFFFF, opacity: 0.42),
        group: Color(hex: 0xFFFFFF, opacity: 0.72),
        hairline: Color(hex: 0x000000, opacity: 0.07),
        selection: Color(hex: 0x000000, opacity: 0.055),
        textPrimary: Color(hex: 0x1B1C20),
        textSecondary: Color(hex: 0x1B1C20, opacity: 0.52),
        textTertiary: Color(hex: 0x1B1C20, opacity: 0.34),
        controlFill: Color(hex: 0xFFFFFF, opacity: 0.80),
        controlBorder: Color(hex: 0x000000, opacity: 0.10),
        accentOverride: Color(hex: 0x3F6AE0)
    )

    /// 蓝黑。
    public static let ink = NativeTheme(
        id: "ink",
        name: "Ink",
        appearance: .dark,
        ground: Color(hex: 0x191B21),
        toolbar: Color(hex: 0xFFFFFF, opacity: 0.05),
        group: Color(hex: 0xFFFFFF, opacity: 0.055),
        hairline: Color(hex: 0xFFFFFF, opacity: 0.08),
        selection: Color(hex: 0xFFFFFF, opacity: 0.075),
        textPrimary: Color(hex: 0xEAECF2),
        textSecondary: Color(hex: 0xEAECF2, opacity: 0.55),
        textTertiary: Color(hex: 0xEAECF2, opacity: 0.34),
        controlFill: Color(hex: 0xFFFFFF, opacity: 0.09),
        controlBorder: Color(hex: 0xFFFFFF, opacity: 0.12),
        accentOverride: Color(hex: 0x7F9CF5)
    )

    public static let lightThemes: [NativeTheme] = [systemLight, overcast]
    public static let darkThemes: [NativeTheme] = [systemDark, ink]

    public static let defaultLightId = systemLight.id
    public static let defaultDarkId = systemDark.id

    public static func theme(id: String, appearance: NativeAppearance) -> NativeTheme {
        let pool = appearance == .dark ? darkThemes : lightThemes
        return pool.first { $0.id == id } ?? (appearance == .dark ? systemDark : systemLight)
    }
}

// MARK: - 外观设置与解析

/// 用户在「通用 → 外观」里选的东西(V4:外观是全 app 的,不长在路由那一屏)。
public struct NativeAppearanceSettings: Sendable, Equatable {
    public var lightThemeId: String
    public var darkThemeId: String
    /// 半透明背景。关掉不落到白底,落到选定主题的 `ground` 上。
    public var translucencyEnabled: Bool
    /// 跟随系统「降低透明度」。
    public var followsReduceTransparency: Bool

    public init(
        lightThemeId: String = NativeThemeCatalog.defaultLightId,
        darkThemeId: String = NativeThemeCatalog.defaultDarkId,
        translucencyEnabled: Bool = true,
        followsReduceTransparency: Bool = true
    ) {
        self.lightThemeId = lightThemeId
        self.darkThemeId = darkThemeId
        self.translucencyEnabled = translucencyEnabled
        self.followsReduceTransparency = followsReduceTransparency
    }
}

/// 解析结果:这一刻到底用哪套 token、要不要铺材质。
public struct NativeResolvedAppearance: Sendable, Equatable {
    public let theme: NativeTheme
    /// 为 false 时窗口用 `theme.ground` 铺不透明底,而不是 `.regularMaterial`。
    public let usesMaterial: Bool
}

extension NativeAppearanceSettings {
    /// V2 的落点:系统「降低透明度」开启时**自动切到不透明主题**,
    /// 而不是把界面降级成一块纯灰。这条让无障碍分支变成了产品功能。
    public func resolve(
        appearance: NativeAppearance,
        systemReduceTransparency: Bool
    ) -> NativeResolvedAppearance {
        let theme = NativeThemeCatalog.theme(
            id: appearance == .dark ? darkThemeId : lightThemeId,
            appearance: appearance
        )
        let material: Bool
        if followsReduceTransparency && systemReduceTransparency {
            material = false
        } else {
            material = translucencyEnabled
        }
        return NativeResolvedAppearance(theme: theme, usesMaterial: material)
    }
}
#endif
