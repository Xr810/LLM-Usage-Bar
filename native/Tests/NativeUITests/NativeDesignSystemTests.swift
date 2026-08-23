#if os(macOS)
import AppKit
import SwiftUI
import Testing
@testable import NativeUI

/// 把 Color 拍成 sRGB 分量,好做真正的比较。
/// 只对**字面色**(身份色、状态色)用它 —— 系统语义色是动态的,在测试里取到的分量
/// 只代表某一种外观,拿来断言没有意义。
private func rgba(_ color: Color) -> [Int] {
    let ns = NSColor(color).usingColorSpace(.sRGB)!
    return [ns.redComponent, ns.greenComponent, ns.blueComponent, ns.alphaComponent]
        .map { Int(($0 * 255).rounded()) }
}

// MARK: - 身份色

@Test func identityPaletteHasSixColorsAndWrapsAround() {
    #expect(NativeIdentityPalette.colors.count == 6)
    // 第 7 家回到头,负数索引也不能崩。
    #expect(NativeIdentityPalette.color(forIndex: 6) == NativeIdentityPalette.color(forIndex: 0))
    #expect(NativeIdentityPalette.color(forIndex: 7) == NativeIdentityPalette.color(forIndex: 1))
    #expect(NativeIdentityPalette.color(forIndex: -1) == NativeIdentityPalette.color(forIndex: 5))
}

@Test func identityColorsNeverCollideWithStatusColors() {
    // 这是视觉方向里的硬规则:green / yellow / red 保留给状态,身份色必须避开。
    // 撞了的话「绿=好」这类语义就会被一个恰好是绿色的 provider 破坏。
    let statusColors = [
        NativeStatusColor.ok, NativeStatusColor.warning, NativeStatusColor.destructive,
    ]
    for appearance in NativeAppearance.allCases {
        let status = Set(statusColors.map { rgba($0.resolved(for: appearance)) })
        for identity in NativeIdentityPalette.colors {
            #expect(!status.contains(rgba(identity.resolved(for: appearance))))
        }
    }
}

@Test func identityColorsDifferBetweenLightAndDark() {
    // 身份色不是一个常量:深色下要换变体,否则在深底上发闷。
    for pair in NativeIdentityPalette.colors {
        #expect(rgba(pair.light) != rgba(pair.dark))
    }
}

@Test func identityColorsAreAllDistinct() {
    let light = Set(NativeIdentityPalette.colors.map { rgba($0.light) })
    #expect(light.count == NativeIdentityPalette.colors.count)
}

// MARK: - 状态色

@Test func warningGlyphIsDarkNotWhite() {
    // 黄底太亮,白笔画看不清 —— 苹果自己也是取深色笔画。
    let glyph = rgba(NativeStatusColor.warningGlyph)
    #expect(glyph[0] < 40 && glyph[1] < 40 && glyph[2] < 40)
    #expect(glyph[3] < 255)  // 是半透明黑,不是纯黑
}

@Test func statusColorsUseRealSystemValues() {
    // 对齐 macOS 系统色的真实取值,不是自己调的近似色。
    #expect(rgba(NativeStatusColor.ok.light) == [40, 205, 65, 255])
    #expect(rgba(NativeStatusColor.ok.dark) == [50, 215, 75, 255])
    #expect(rgba(NativeStatusColor.warning.light) == [255, 204, 0, 255])
    #expect(rgba(NativeStatusColor.destructive.light) == [255, 59, 48, 255])
}

// MARK: - 主题

@Test func systemThemeFollowsTheSystemAccentAndIsTheDefault() {
    // 视觉方向 V3:强调色由主题定,但内置一套跟随 accentColor 的「系统」主题,默认选它。
    #expect(NativeThemeCatalog.systemLight.accentOverride == nil)
    #expect(NativeThemeCatalog.systemDark.accentOverride == nil)
    #expect(NativeThemeCatalog.defaultLightId == "system")
    #expect(NativeThemeCatalog.defaultDarkId == "system")
}

@Test func namedThemesPinTheirOwnAccent() {
    #expect(NativeThemeCatalog.overcast.accentOverride != nil)
    #expect(NativeThemeCatalog.ink.accentOverride != nil)
}

@Test func themeLookupFallsBackToSystemForUnknownIds() {
    let light = NativeThemeCatalog.theme(id: "nonexistent", appearance: .light)
    #expect(light.id == "system")
    #expect(light.appearance == .light)

    let dark = NativeThemeCatalog.theme(id: "nonexistent", appearance: .dark)
    #expect(dark.id == "system")
    #expect(dark.appearance == .dark)

    // 拿浅色 id 去查深色池,同样落回系统主题,而不是返回一套外观不匹配的 token。
    #expect(NativeThemeCatalog.theme(id: "overcast", appearance: .dark).id == "system")
}

@Test func everyCatalogThemeMatchesThePoolItLivesIn() {
    for theme in NativeThemeCatalog.lightThemes { #expect(theme.appearance == .light) }
    for theme in NativeThemeCatalog.darkThemes { #expect(theme.appearance == .dark) }
}

@Test func noThemeGroundIsPureWhiteOrPureBlack() {
    // 纯白/纯黑是「没设计过」的样子。字面主题必须带一点色偏。
    for theme in [NativeThemeCatalog.overcast, NativeThemeCatalog.ink] {
        let ground = rgba(theme.ground)
        #expect(ground[0...2] != [255, 255, 255])
        #expect(ground[0...2] != [0, 0, 0])
        // 而且要真的偏色,不是中性灰:三个分量不能全相等。
        #expect(Set(ground[0...2]).count > 1)
    }
}

// MARK: - 外观解析

@Test func reduceTransparencyDropsMaterialWhenFollowed() {
    // V2:系统开了「降低透明度」就自动切到不透明主题,而不是把界面降级成一块纯灰。
    let settings = NativeAppearanceSettings(translucencyEnabled: true,
                                            followsReduceTransparency: true)
    let resolved = settings.resolve(appearance: .light, systemReduceTransparency: true)
    #expect(!resolved.usesMaterial)
}

@Test func reduceTransparencyIsIgnoredWhenNotFollowed() {
    let settings = NativeAppearanceSettings(translucencyEnabled: true,
                                            followsReduceTransparency: false)
    let resolved = settings.resolve(appearance: .light, systemReduceTransparency: true)
    #expect(resolved.usesMaterial)
}

@Test func translucencySwitchWinsWhenSystemIsNotReducing() {
    let off = NativeAppearanceSettings(translucencyEnabled: false)
    #expect(!off.resolve(appearance: .light, systemReduceTransparency: false).usesMaterial)

    let on = NativeAppearanceSettings(translucencyEnabled: true)
    #expect(on.resolve(appearance: .light, systemReduceTransparency: false).usesMaterial)
}

@Test func resolutionPicksTheThemeMatchingTheCurrentAppearance() {
    let settings = NativeAppearanceSettings(lightThemeId: "overcast", darkThemeId: "ink")
    #expect(settings.resolve(appearance: .light, systemReduceTransparency: false).theme.id
        == "overcast")
    #expect(settings.resolve(appearance: .dark, systemReduceTransparency: false).theme.id == "ink")
}

// MARK: - 刻度

@Test func radiiAreFourAscendingSteps() {
    let steps = [
        NativeRadius.small, NativeRadius.medium, NativeRadius.large, NativeRadius.window,
    ]
    #expect(steps == steps.sorted())
    #expect(Set(steps).count == 4)
}

@Test func spacingFollowsTheEightPointGridWithOneHalfStep() {
    let steps = [
        NativeSpacing.xxs, NativeSpacing.xs, NativeSpacing.sm,
        NativeSpacing.md, NativeSpacing.lg, NativeSpacing.xl,
    ]
    #expect(steps == steps.sorted())
    // HIG 计划 §1.4 定的是 4/8/12/16/20/24 ——「8pt 网格 + 半档」意思是每档 4pt,
    // 所以 4/12/20 都是半档。判据是:每一档都必须是 4 的整数倍,且没有重复。
    #expect(steps.allSatisfy { $0.truncatingRemainder(dividingBy: 4) == 0 })
    #expect(Set(steps).count == steps.count)
    // 落在整 8pt 网格上的那三档也要在
    #expect(steps.filter { $0.truncatingRemainder(dividingBy: 8) == 0 } == [8, 16, 24])
}
#endif
