# T31:面板的编辑入口(sheet)

创建:2026-08-24
分支:`feat/native-bridge-replant`(接在 T30 之后)

---

## 0. 背景

T30 之后面板是**只读**的:「添加 Provider」「编辑映射」「绑定凭据」三个按钮都是空回调。
本任务把前两个做通,第三个卡在缺 bridge 通道上(见 §3)。

## 1. Provider 编辑器

`ProviderEditorSheet`。字段:标识、显示名、Base URL、协议、凭据方式、启用。

**刻意没有 priority。** 顺序归「模式与分账」那一节 —— 一件东西只能有一个编辑入口,
两处都能改同一个值的话,用户永远不知道哪个说了算。

**标识建好之后锁死**,并在界面上写明理由(改它等于换一家,已有的映射和分账会跟着断)。
本地校验:标识非空、不重复、无空格;Base URL 非空且以 http(s) 开头。
后端仍然会拒非法的 `wire_api` / `auth_kind`,但这里用 Picker,构造不出非法值。

## 2. 模型映射编辑器

`ModelRoutesEditorSheet`。写侧是**全量替换**,所以编辑的是整份清单。

**决定 11 的落点:粘贴清单和「添加一行」并排,不是藏在角落的次要入口。**
有些 provider 永远拉不到 `/v1/models`,手动这条路必须一直好用。

解析器 `ModelRoutePaste` 单独成文件、单独测(6 条),接受:

```
gpt-5.6 = sol-gpt-5.6-1120      等号
gpt-5.6 -> sol-gpt-5.6-1120     箭头(-> 或 →)
gpt-5.6 : sol-gpt-5.6-1120      冒号
gpt-5.6, sol-gpt-5.6-1120       逗号
gpt-5.6 <TAB> sol-gpt-5.6-1120  制表符
gpt-5.6                         只有一个名字 → 上游同名
```

空行与 `#` 注释行忽略;引号和多余空白剥掉;**重复的逻辑模型保留第一条** ——
静默用最后一条覆盖会让用户以为自己写的那条生效了。

粘贴框给两个动作:「追加到上面」与「替换全部」。全量替换是个破坏性动作,
所以它是一个明确的按钮,不是粘贴之后的默认行为。

## 3. 凭据入口:卡在缺通道上,**本任务没做**

绑定 API Key 需要往钥匙串写,而 bridge 的十个路由命令里没有凭据相关的。
仓库里已有的是 **tauri 命令**(`list_provider_api_keys` / `create_provider_api_key` /
`rename_provider_api_key` / `delete_provider_api_key`,在 `api/commands/usage_dashboard.rs`),
它们**没有被 bridge 暴露**。

所以要做凭据 sheet,得先加两个 bridge 命令(至少 `listProviderApiKeys` 与
`createProviderApiKey`)。**没有硬凑一个存不进去的界面** —— 一个点了没反应的绑定框
比没有这个框更糟。

面板上「绑定…」「解绑」目前仍是外部回调,由宿主决定怎么接。

## 4. 面板的回调收敛

T30 时 `RouterPanelView` 有八个注入回调。现在只剩三个:
`onBindCredential` / `onUnbindCredential`(等 §3)、`onRevealInFinder`(要 NSWorkspace)。
其余动作面板自己调 model —— 它本来就持有 model,绕一圈注入没有意义。

## 5. 验证与看不到的

`swift build` 通过;`swift test` **66 passed**;设计 lint 三条全绿。

**两个 sheet 的渲图只能验骨架。** `ImageRenderer` 渲不了 `TextField` / `Picker` /
`Toggle`,它们会出黄色占位块 —— 渲图能确认标签位置、分隔线、锁定提示、默认按钮是蓝的,
**字段本身验不了**。两个 sheet 各带三个 `#Preview`(默认 / 边界态 / 深色),
在 Xcode canvas 里看。
