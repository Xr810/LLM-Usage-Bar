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

## 3. 验证与**没有**验证的

- `swift build` 通过;`swift test` **58 passed**(45 + 13 条 model 测试)
- `script/lint_design.sh` 三条全绿

**视觉没有被验证过。** 我请求截屏权限时被拒,所以这四屏我一行都没亲眼看过 ——
只知道它编译通过、逻辑测试通过。**看起来对不对是未知的**,不要把本任务当作视觉已定稿。

看的办法:

```
cd native && swift run RouterPanelPreview [--theme system|overcast|ink] [--opaque] [--empty]
```

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
