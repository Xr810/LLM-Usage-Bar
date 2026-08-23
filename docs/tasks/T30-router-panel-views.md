# T30:路由面板四个分区

创建:2026-08-24
分支:`feat/native-bridge-replant`(接在 T29 之后)
依据:`docs/design/2026-08-23-router-panel-visual-direction.md`

---

## 0. 形态

**一个可滚动 pane 里从上往下四个分区**,不是四个页面、也不是步骤侧栏。
它们是一条有依赖的启用链,位置本身就表达了先后;设置窗里再套一层导航是网页习惯。

外观(主题、半透明)**不在这里** —— 那是全 app 的设置,归「通用」分区(V4)。

## 1. 文件

```
Sources/NativeUI/Components/NativeRowComponents.swift   分组表单的基本件
Sources/NativeUI/Router/RouterPanelModel.swift          状态(四分区共用一个)
Sources/NativeUI/Router/ModelMappingSection.swift       一、模型映射
Sources/NativeUI/Router/CredentialsSection.swift        二、凭据
Sources/NativeUI/Router/PointerTakeoverSection.swift    三、接管 Codex
Sources/NativeUI/Router/ModeAndAccountingSection.swift  四、模式与分账
Sources/NativeUI/Router/RouterPanelView.swift           装配 + 预览 + 假数据源
Sources/RouterPanelPreview/main.swift                   跑起来看一眼用的壳
```

## 2. 设计里那些「不能妥协」的条款,落在代码哪儿

| 条款 | 落点 |
| --- | --- |
| 空态直接说后果(「路由现在对所有请求返回 503」),不说「暂无数据」 | `ModelMappingSection.emptyState` |
| key 存进钥匙串后不可读 —— 没有密码框、没有眼睛图标,只有「已绑定 / 替换 / 解绑」 | `CredentialsSection.actions` |
| 接管前把将写入 `~/.codex/config.toml` 的原文摊开(删除行带删除线、新增行带变更条、备份承诺) | `PointerTakeoverSection.diffPreview` |
| 点完必须提示重启 Codex,且是**常驻横条**不是 toast(决定 34) | `PointerTakeoverSection.restartBanner` + `model.needsCodexRestart` |
| 手动模式**持续可见**(决定 31) | `ModeAndAccountingSection.manualBanner` |
| 模式切换器换的是**下面那块区域本身**:自动 = 排序列表,手动 = 点选列表 | `orderingList` / `pickingList` |
| 被拉黑那家手动模式下**照样可选**;缺凭据那家才真的不可选(决定 29) | `pickingRow.selectable` |
| 分账是**下界不是账单**,表头 `输入 ≥ / 输出 ≥` + 说明 | `accountingHeader` + `accounting` 里那段文字 |
| 分账空态把「没重启 Codex 就不会记账」接上 | `emptyAccounting` |
| 身份色在映射表 / 凭据 / 排序 / 分账**四处一致** | `model.identityIndex(forProvider:)` |
| 错误钉在顶部不自动消失 | `NativeErrorBanner` + `model.errorMessage` |

## 3. 怎么看,以及看到了什么

### 两条通道

**Xcode 预览**:打开 `native/Package.swift` → `RouterPanelView.swift` → Editor ▸ Canvas。
**scheme 必须选 `NativeUI`,不能选带可执行 target 的那个** —— 详见 §3.1。

**无头渲图**(不需要 Xcode、不需要屏幕权限):

```
PANEL_SHOTS_DIR=/tmp/shots swift test --filter renderPanelSnapshots
```

七张:默认 / Overcast / Ink 深色 / 空态 / 手动模式 / 接管后 / 错误条。

### 3.1 踩过的两个坑,都记在这

**一、可执行 target 会把 SwiftUI 预览堵死。**
最初把渲染器做成了可执行 target `RouterPanelPreview`,结果 Xcode 挑它当预览宿主,
报 `DebugDylibNotEnabled: 需要把 ENABLE_DEBUG_DYLIB 设为 YES` —— 而 SwiftPM 设不了
这个构建设置,四个 `#Preview` 全部渲染失败。**已把渲染器搬进测试 target**(测试不会
被选作预览宿主),包里现在没有任何可执行 target。

**二、`ImageRenderer` 渲不出 `ScrollView` 里的内容。**
实测:`Text` 与 `VStack` 正常,`ScrollView` 渲出来非背景像素为 **0**(整屏空白)。
所以 `RouterPanelView` 把内容层拆成了 `.content`(不含 ScrollView),渲染器渲那一层,
真实界面照旧带滚动。

### 3.2 这条通道**看不到**什么

`ImageRenderer` 渲不了 AppKit 支撑的控件:**模式那个 segmented `Picker` 和尝试顺序的
`List` 会渲成黄色禁止符号占位块**。那不是界面坏了,是这条通道看不见它们 ——
**这两个控件必须在 Xcode 预览或真窗口里验**,渲图结果对它们无效。

### 3.3 靠渲图逮到并修掉的两处

1. **已停用 provider 的映射行没跟着变暗** —— `.opacity` 只加在了表头行上,底下的映射
   仍然全亮,看起来像在生效。这一屏最不该给错的就是这个信息。已改成盖住整块
2. **「解绑」渲成了粉红实心块** —— 手工 `.foregroundStyle(destructive)` 的结果。
   改用系统的 `Button(role: .destructive)`

### 3.4 现在的状态

`swift build` 通过;`swift test` **59 passed**;`script/lint_design.sh` 三条全绿;
七张渲图逐张看过,除 §3.2 那两个控件外均符合设计。

## 4. 值得单说的两条测试

- `failedTakeoverDoesNotClaimARestartIsNeeded` —— 接管**失败**时不能立重启横条。
  否则用户重启完发现根本没接管,比不提示更糟
- `errorBannerDoesNotClearItself` —— 后续一次成功的读不能把错误条冲掉。
  设计要求是「用户点掉才消失」

## 5. 遗留

- **没有产品外壳**:菜单栏、窗口、生命周期、与 bridge 的真实连接都还没有。
  `RouterPanelPreview` 只是个看图的壳,数据来自 `PreviewRouterRepository`
- 添加 provider、编辑映射、绑定凭据这三个入口目前是**回调空实现** —— 需要各自的 sheet
- 菜单栏 popover(设计里的 4c)未做
- 德语长文案压力测试未做(布局用的是 `fixedSize` 与 `frame(maxWidth:)`,理论上能换行,
  但没实测)
