# 原生壳 HIG 改造计划(SwiftUI 视觉与交互)

> **历史设计，路线已变更（2026-09-17）：** SwiftUI 相关方案已停止，当前采用 React + TypeScript / Tauri 2 + Rust 支持 macOS 与 Windows。以仓库 README 和 HANDOFF 顶部更新为准。


创建:2026-08-13
适用分支:`feat/swift-native-shell`(PR #27)
前置结论:`docs/design/tech-route-review-2026-08-12.md` §4、HANDOFF §11.8 —— 方案 B,
Rust 核心 + SwiftUI 壳,绞杀者节奏。

> **这份计划只改 UI 层。** 不动 bridge 协议、不动 Rust、不动数据库、不动 `UsageCore`
> 的任何 DTO。所有改动集中在 `native/Sources/LLMUsageBarNative/` 下 14 个文件里,
> 契约测试(`native/Tests/UsageCoreTests/`)应当**一行不改地继续通过** —— 这是本次
> 改造范围没有跑偏的最强判据。

---

## 1. 为什么要做

Swift 壳已经落地(30 个 `.swift`,菜单栏 / 主窗口 / 设置三个场景都通了),但它是
**把 React 界面逐行翻译成了 SwiftUI**,不是按 macOS 的规矩重写的。实测统计:

| 现象 | 计数 | 问题 |
| --- | --- | --- |
| `.font(.system(size: N))` 写死字号 | **90 处** | 用的是 9/10/11/12/18/19/25pt,苹果的刻度是 10/11/12/13/15/17/22/26,没有一个对得上 |
| `NativePalette` 字面 RGB 色 | 29 处引用 | 靛蓝 `#5A4FDE` + 米白 `#F8F7F4` 背景 —— 这是 Tailwind 色板搬过来的,不是 macOS 的中性灰 |
| `cornerRadius` 非 `.continuous` | 11 / 18 | 普通圆弧,一眼看出是 Web |
| `.onHover` | **1 处** | 整个 app 几乎没有 hover 反馈,Mac 用户会觉得"死的" |
| 自定义 `ButtonStyle` | **0 个** | 全靠 `.buttonStyle(.plain)` 手搓背景,按下态没有统一处理 |
| `NativePalette.primary(.light)` 硬传 `.light` | 2 处 | `MainWindowView.swift:133`、`NativeSettingsView.swift:59` —— 深色模式下也用浅色主色 |
| 主窗口全局 `.controlSize(.small)` | 1 处 | 苹果只在检查器/浮动面板用 small,主窗口用 regular |

**目标:把这层"Web 味"换成系统同源的观感,同时保住现有的信息层次和自绘卡片。**

原则(上游讨论结论):**标准交互用系统控件,展示性内容自绘但用系统的材质和 token。**
苹果自家的天气 / 股票 / 控制中心也是自绘卡片,所以自绘不是问题;写死颜色和字号才是。

### 一处需要更正的前提

macOS **没有 iOS 那样的 Dynamic Type**,语义字号不会随用户的字号设置缩放。所以改用
`.body` / `.callout` 的理由不是"自动缩放",而是:**它们就是苹果自己那套字号刻度**,
用了才能和其他 Mac app 的层次对齐。缩放这条收益在 macOS 上不成立,别拿它当验收项。

---

## 2. 阶段划分

五个阶段,**每阶段独立可合并、可回退**,不要攒成一个大 PR。

### P0 — 基线与护栏(约半天)

1. 从 `feat/swift-native-shell` 开 `feat/native-hig-restyle`。
2. **截图基线**:改造前把菜单栏 / 主窗口三个维度 / 设置三个 pane,浅深各截一张,
   存 `docs/assets/hig-restyle/before/`。后面每阶段出 after 对照。
3. **写 lint 脚本** `native/script/lint_design.sh`,CI 里跑,三条硬规则:
   - 禁止 `\.font\(\.system\(size:` (例外:`ProviderActivityHeatmap` 里按格子尺寸
     算出来的字号,加 `// swiftlint:disable` 式的显式豁免注释)
   - 禁止不带 `style: .continuous` 的 `cornerRadius:`
   - 禁止 `NativeDesignSystem.swift` 以外的文件出现 `Color(red:`

   > 这是本计划里**唯一适合派给 Codex 的任务**(纯脚本 + CI 接线,无审美判断)。

**验收:** lint 在当前代码上跑出 90 + 11 + N 条失败——先让它红着,后面逐阶段清零。

---

### P1 — token 层重建(`NativeDesignSystem.swift`,235 行)

这是整个计划的地基,**必须一次做完再往下走**,否则后面每个视图都要返工。

#### 1.1 颜色 —— 删掉 `NativePalette` 的字面 RGB

| 现有 | 换成 | 说明 |
| --- | --- | --- |
| `background()` 米白/近黑 | `Color(nsColor: .windowBackgroundColor)` | macOS 是中性灰,不是暖白。米白是最大的"这是网页"的破绽 |
| `card()` | 保留 `.regularMaterial`(已在用) | 这块本来就对 |
| `primary()` 靛蓝 | `Color.accentColor` | 跟随用户在系统设置里选的强调色 |
| `border()` | `Color(nsColor: .separatorColor)` | 自带增强对比度响应,可删掉手写的 `highContrast` 分支 |
| `recessed()` | `Color(nsColor: .unemphasizedSelectedContentBackgroundColor)` 或 `.quaternary` | |
| `status()` 红黄绿 | **保留自定义** | 语义色里没有对应角色。但必须:①过一遍增强对比度;②**配 SF Symbol 形状区分**,满足「不使用颜色传达信息」辅助功能 |

同时修掉两处硬传 `.light` 的调用。

#### 1.2 字号 —— 90 处映射到语义 text style

macOS 的语义刻度(用这张表逐处替换,不要自由发挥):

| 语义 | pt | 用在 |
| --- | --- | --- |
| `.largeTitle` | 26 | 不用 |
| `.title` | 22 | 主窗口大数字(现在的 25pt) |
| `.title2` | 17 | 卡片主数值(现在的 17pt,已对) |
| `.title3` | 15 | — |
| `.headline` | 13 semibold | 区块标题(现在的 18/19pt 全部降到这里) |
| `.body` | 13 | 正文 |
| `.callout` | 12 | 次级正文(现在的 12pt) |
| `.subheadline` | 11 | 行内标签(现在的 11pt) |
| `.footnote` | 10 | 辅助说明(现在的 10pt) |
| `.caption2` | 10 | 角标 |

**现有的 9pt 全部提到 `.footnote`(10pt)。** 9pt 低于苹果任何一个刻度,在 Retina 上
也偏糊,这是 Web 密度思维的残留。

数字保留 `.monospacedDigit()`(现有 26 处,做得对,继续)。

#### 1.3 圆角 —— 收敛成 4 档,全部 continuous

```
NativeRadius.small  = 6   // 角标、小按钮
NativeRadius.medium = 10  // 行、输入框
NativeRadius.large  = 14  // 卡片
NativeRadius.window = 16  // 弹出层
```

现在散落着 6/7/8/12 四个值 + `size * 0.28` 一个算式,统一到上面四档。

#### 1.4 间距

现有的 3/4/6/9/10/13/14/15/16/20/22 收敛成 4/8/12/16/20/24(8pt 网格 + 半档)。

**P1 交付:** `NativeDesignSystem.swift` 重写;lint 三条规则全绿;
14 个视图文件因为改字号/改色而产生的机械改动一并落在这个 PR 里。
**验收:** Xcode Canvas 六个预览 × 浅/深 × 增强对比度 = 24 张,与 before 对照;
把系统强调色改成橙色和石墨,确认全局跟随。

---

### P2 — 设置窗口改用系统 `Form`(`NativeSettingsView.swift`,262 行)

**这是全 app 唯一应该交给系统控件的地方。** 用户对设置窗口的观感期待最强,而
grouped Form 的行高、分隔线内缩、label 列宽协商这些细节,自绘做不像。

现状是手搓的:190pt 固定宽侧栏 + `Button` 列表 + `ScrollView`。

改法:

```swift
Settings {
    TabView {
        GeneralPane().tabItem { Label(…, systemImage: "gearshape") }
        ProvidersPane().tabItem { Label(…, systemImage: "server.rack") }
        DiagnosticsPane().tabItem { Label(…, systemImage: "stethoscope") }
    }
}
```

每个 pane 内部用 `Form { Section { … } }.formStyle(.grouped)`。

> **为什么 TabView 而不是侧栏:** 三个 pane 用侧栏偏重,那是「系统设置」的结构;
> 第三方 Mac app 的设置窗口(Xcode、Things、Fantastical)几乎都是顶部图标 TabView。
> 三个及以下用 TabView,超过五个再考虑侧栏。

同时:**去掉固定宽度**,设置窗口应当由内容撑开、随 pane 切换动画调整尺寸——这是
Mac 设置窗口的标志性行为,固定宽度一眼假。

**验收:** 与「系统设置 → 通用」并排截图;四语言下 label 列不溢出(德语最长)。

---

### P3 — 主窗口与 breakdown 行

`MainWindowView.swift`(280)、`ProviderMonitoringView.swift`(530)、
`BreakdownDashboardViews.swift`(302)、`DashboardDetailViews.swift`(237)。

1. **去掉全局 `.controlSize(.small)`**,改回 regular;只在密集的 breakdown 区域局部用 small。
2. **header 并入 toolbar**。现在是自绘的 header 条 + `Divider`;Mac 的做法是标题和
   范围切换器都放 `.toolbar`,内容区直接顶到窗口边。
3. **breakdown 行保持自绘**(结构上它是带 share 染色背景的 disclosure row,`Table`
   做不出来),但要补齐 `List` 免费给的行为:
   - `.onHover` 加克制的 hover 底色(5% fill,不是 Web 那种明显高亮)
   - `@FocusState` + `.focusable()` + `onKeyPress(.upArrow/.downArrow/.space)`
   - `.accessibilityElement(children: .combine)` + 一句完整的 label(现在只有 7 处 a11y 标注)
   - 统一的自定义 `ButtonStyle`(全项目 0 个),把按下态收口到一处
4. `LazyVGrid(.adaptive(minimum: 390, maximum: 620))` 的卡片栅格保留,只调间距到 P1 刻度。

**验收:** 纯键盘从窗口打开走到某个 provider 详情再返回,全程不碰鼠标;
VoiceOver 逐行朗读一条 breakdown 行,读出来是一句完整的话而不是散字段。

---

### P4 — 菜单栏 popover 打磨(`UsageMenuView.swift`,319 行)

**这是用户看得最多的界面,单独一轮。** 对标控制中心和系统的 Wi-Fi / 电池菜单:

- 宽度对齐系统菜单的观感(不要 Web 卡片的宽松内边距)
- 分组用细分隔而不是卡片嵌套 —— popover 里套卡片是 Web 习惯
- 每行 hover 高亮 + 整行可点
- `.presentationBackground(.regularMaterial)`,尊重"降低透明度"
- 状态图标用 SF Symbols 的 variable color,进度感直接体现在图标上

**验收:** 与系统电池菜单、控制中心并排截图。

---

### P5 — 全量验收

矩阵:**四语言 × 浅/深 × {标准, 增强对比度} × {标准, 降低透明度} × {标准, 降低动态效果}**。

外加:

- VoiceOver 全流程走查
- 系统强调色切三种(蓝 / 橙 / 石墨),全局跟随无残留硬编码色
- 「不使用颜色传达信息」开启后,红黄绿状态仍可区分(靠 P1.1 的形状区分)
- 与现有 React 版并排截图,确认信息层次没有在改造中丢失

---

## 3. 分工

按全局准则,UI / 样式 / 视觉层次归 Claude 自己做,**这份计划里 P1–P5 全部属于此类,
不外派。** 唯一适合派给 Codex 的是:

- P0 的 `lint_design.sh` + CI 接线
- P5 的自动化截图脚本(如果决定做)

也就是说,这次基本不需要走委派流程。

## 4. 明确不做的事

- 不动 bridge 协议、Rust 侧、数据库、`UsageCore` 里任何 DTO
- 不动 `native/Tests/UsageCoreTests/` —— 这些测试必须原样通过
- 不改 Preview / Production 的 bundle identity 和切换时机(那是迁移路线的事)
- 不引入任何第三方 SwiftUI 库
- 不为了"苹果风"删减现有信息 —— 层次可以变,数据不能少

## 5. 开工前需要拍板的三件事

1. **品牌靛蓝 vs 系统强调色。** 全面改用 `Color.accentColor` 意味着放弃 `#5A4FDE`
   这个品牌色,UI 会跟随用户的系统设置变色。
   **建议:交互元素(选中、按钮、进度)跟随系统;品牌色只保留在 app 图标和关于页。**
   这是"很苹果"最核心的一次取舍,也是最容易反悔的一个。
2. **设置窗口 TabView(建议)还是侧栏。** 见 P2。
3. **在 `feat/swift-native-shell` 上继续,还是开 `feat/native-hig-restyle`。**
   **建议开新分支**,因为 PR #27 还没合,叠改造会让 review 变成不可能。
