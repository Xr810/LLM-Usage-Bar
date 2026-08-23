# T29:token 层(主题结构)

创建:2026-08-24
分支:`feat/native-bridge-replant`(接在 T28 之后)
依据:`docs/design/2026-08-23-router-panel-visual-direction.md`、
`docs/design/2026-08-13-native-hig-restyle-plan.md` §P0/§P1

---

## 0. 做了什么

`native/Sources/NativeUI/NativeDesignSystem.swift` —— 从零写,不是改旧的:
旧的那份在没有移栽过来的 14 个视图里,而且它的取值(米白底、字面靛蓝、9pt)
正是这轮要换掉的东西。

## 1. 为什么单独一个 target

`NativeUI` 与 `UsageCore` 平级。**UsageCore 是数据与传输层,不该被 SwiftUI 拖进去** ——
它现在只 import Foundation,加一层 SwiftUI 依赖会让它在别的场景(命令行探针、
将来的其它前端)白背一个 UI 框架。视图将来长在可执行 target 里,依赖 NativeUI 拿 token。

## 2. 主题结构

一套主题定义**底色、面、三级文字、分隔、选中态、强调色**。
**不包含状态色与身份色** —— 那两组跨主题保持一致,否则「绿 = 好」这类语义会随主题漂移。

| 主题 | 外观 | 强调色 |
| --- | --- | --- |
| `system` | 浅 / 深各一套 | **`nil` = 跟随 `Color.accentColor`。默认就是它**(V3) |
| `overcast` | 浅 | 自带 |
| `ink` | 深 | 自带 |

中性色在 `system` 主题里走 AppKit 语义色(`.windowBackgroundColor` /
`.separatorColor` / `.labelColor` …),字面主题才写死取值。

`NativeAppearanceSettings.resolve(appearance:systemReduceTransparency:)` 是 V2 的落点:
**跟随系统「降低透明度」时自动切到不透明主题**,而不是把界面降级成一块纯灰。

## 3. 刻度

- 间距 `4/8/12/16/20/24`(每档 4pt)
- 圆角四档 `6/10/14/16`,全部 `.continuous`,配 `NativeRadius.shape(_:)`
- 字号只用 macOS 语义刻度,**下限 `.footnote`(10pt)** —— 9pt 低于苹果任何一档
- 身份色六档(indigo/orange/purple/teal/pink/brown),按注册顺序分配、第 7 家回头循环

## 4. lint(P0)

`native/script/lint_design.sh`,三条硬规则:禁止 `.font(.system(size:`、
圆角必须 `.continuous`、字面颜色只能出现在设计系统文件里。注释行不算违规。

**已反向验证**:故意塞一处 `Color(red:` 进去,lint 退出 1 并指出行号 —— 一个永远
通过的 lint 等于没有。

## 5. 验证

`cd native && swift test`:**45 passed**(28 UsageCore + 17 NativeUI)。

值得单说的两条测试:

- `identityColorsNeverCollideWithStatusColors` —— 视觉方向里那条硬规则(green/yellow/red
  保留给状态,身份色必须避开)靠肉眼看不出来,这里把颜色拍成 sRGB 分量真比
- `noThemeGroundIsPureWhiteOrPureBlack` —— 顺带断言三个分量不全相等,即底色必须
  **真的带色偏**。纯中性灰读起来像没设计过,那正是前三轮"太素"的来源之一

写测试时发现自己的断言写错过一次(以为 4 是唯一半档,实际 4/12/20 都是),
按 HIG 计划 §1.4 的原文改正 —— 记在这里,免得以后又按错的理解去收敛间距。

## 6. 下一步

四屏实现。token 有了、bridge 通了、DTO 有了,视图是纯 UI 工作,不外派。
