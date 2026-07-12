# 动态订阅模块与统一 API 用量页设计

## 文档状态

- 状态：已批准（2026-07-13，用户要求按本计划实施前端）
- 日期：2026-07-12
- 目标仓库：`/Users/max/LLM Usage Bar`
- 目标产品：LLM Usage Bar
- 基线规范：`docs/superpowers/specs/2026-07-10-usage-dashboard-design.md`

本文细化并替换基线规范中的首页信息架构和 Provider 配置入口，不改变其中已经确认的计费、额度、事件来源与去重边界。

## 背景

当前首页同时展示时间范围、Provider 配置、静态路由、代理控制和用量结果，导致配置能力压过监测结果。原 CC Switch 右上角的 Codex/OpenCode 切换器已经退出主路径，但该位置本身仍适合承载产品级导航。

新的导航语义不是切换正在使用的 Agent，也不是切换代理转发目标，而是切换当前查看的用量模块。默认模块为：

- Codex
- Claude Code
- Kimi Coding Plan
- API

这些名称是首次使用时创建的默认数据，不得作为前端组件中的固定条件。用户可以在设置中新增、改名、排序、隐藏订阅模块，并将订阅 Provider 实例归入相应模块。

## 目标

1. 将首页变为以订阅额度和 Token 用量为核心的监测界面。
2. 保留原版右上角切换器的操作位置和快速切换体验，但将其语义改为动态用量模块导航。
3. 让 GPT Plus、GPT Pro 等订阅计划作为 Codex 模块内的独立 Provider 实例展示，而不是各占一个顶层模块。
4. 将所有按量 Provider 集中到与订阅模块同级的 API 模块，例如 Azure、OpenRouter。
5. 将 Provider、静态路由和代理配置移入设置页，首页不直接暴露转发配置。
6. 保持 Provider 账单、UsageEvent、QuotaSnapshot 和数据来源之间现有的可信边界。

## 非目标

- 不恢复 Provider 快速切换、自动故障转移或代理路由切换功能。
- 顶部模块导航不修改 Codex、Claude Code、OpenCode 等客户端的实时配置。
- 不把不同订阅账号的额度合并为一个未标注的总额度。
- 不把订阅额度与 API 费用混成同一个数值。
- 不用展示模块替代 `productGroupId`、`providerId` 或其他账单身份。
- 本设计不新增预设商城、云同步、MCP、Skills、终端或完整会话管理入口。

## 术语与层级

### 用量模块 `DashboardModule`

顶层导航单位，仅负责首页分区和展示。模块具有稳定 ID、可编辑名称、类型、排序和可见性。

```text
DashboardModule
  id
  name
  kind = subscription | api
  sortOrder
  visible
  isSystem
```

- `subscription` 模块包含一个或多个订阅 Provider 实例。
- `api` 模块聚合所有启用的按量 Provider。
- 系统只允许一个 `api` 模块。它可排序和改显示名，但不可删除或改为订阅类型。
- 用户可以创建、改名、排序、隐藏或删除自定义订阅模块。
- 删除仍包含 Provider 的订阅模块前，必须先将这些 Provider 移动到其他模块或一并停用；禁止静默丢失归属。

### Provider 实例

一个可独立计费、独立查询额度的账号或路由。Provider 仍是账单和数据归属的基本单位。

示例：

```text
Codex 模块
  GPT Plus（个人）     Provider A
  GPT Pro（工作）      Provider B

Kimi Coding Plan 模块
  Kimi 主账号          Provider C

API 模块
  Azure                Provider D
  OpenRouter           Provider E
```

两个 GPT Plus 账号必须是两个 Provider 实例。额度刷新、失败状态、Token 和事件明细不得跨实例合并。

### `productGroupId`

`productGroupId` 继续表示 UsageEvent 写入时的不可变产品归因，不承担导航模块配置。模块改名、排序或 Provider 展示归属变化不得回写历史事件。

聚合顺序必须是：先按不可变 `providerId` 和 `productGroupId` 查询可信事件，再把结果投影到当前展示模块。模块层不能参与费用去重、事件所有权或额度归属判断。

## 默认数据与动态配置

首次创建新数据库时写入以下模块记录：

| 默认顺序 | 名称             | 类型         | 系统模块 |
| -------- | ---------------- | ------------ | -------- |
| 1        | Codex            | subscription | 否       |
| 2        | Claude Code      | subscription | 否       |
| 3        | Kimi Coding Plan | subscription | 否       |
| 4        | API              | api          | 是       |

默认记录只用于初始化。前端必须从数据库读取模块列表，不得通过模块名称决定组件、额度来源或业务逻辑。

现有数据库升级时：

1. 创建默认模块，但只为存在对应 Provider 的订阅模块自动建立归属。
2. 对现有订阅 Provider，优先依据明确的 `productGroupId` 或已保存来源映射到 Codex、Claude Code、Kimi Coding Plan；无法可靠映射时，仅在确有需要时创建“其他订阅”模块并放入其中，同时标记为“待归类”，由用户在设置中选择。
3. 所有 `billingKind = metered` 的 Provider 自动进入 API 模块。
4. 不修改现有 UsageEvent、QuotaSnapshot、Provider ID、静态路由或 Session 来源绑定。

## 首页信息架构

### 应用顶栏

应用名称保留在左侧。动态模块切换器位于原版 Codex/OpenCode 切换器所在的右上区域，设置入口位于其后。

```text
LLM Usage Bar             [ Codex | Claude Code | Kimi Coding Plan | API ] [设置]
```

- 模块按 `sortOrder` 显示，只渲染 `visible = true` 的模块。
- 当前模块使用明确的选中状态，但点击只改变页面内容，不触发 Provider、CLI 配置或代理路由变更。
- 用户新增订阅模块后，导航自动出现新模块，不需要发版或修改代码。
- 模块过多时保持横向滚动，并提供“更多”菜单；不得为了塞入一行而把文字压缩到不可读。
- 记住上次选中的模块。若该模块被隐藏或删除，则回退到第一个可见模块；没有可见订阅模块时回退到 API。
- 移除首页中重复的“用量仪表盘”标题和说明，只保留应用壳中的一份产品身份。

### 订阅模块页面

订阅模块分为 Provider 实例选择和当前实例详情两层。

当模块只有一个启用 Provider 时，直接显示详情，并在标题处标明 Provider 名称。当模块有多个 Provider 时，显示二级实例切换器：

```text
Codex
[ GPT Plus（个人） | GPT Pro（工作） ]
```

实例详情按以下顺序展示：

1. Provider 名称、套餐标签、数据来源和最近成功刷新时间。
2. 5 小时额度卡：已用或剩余百分比、重置时间、倒计时和状态。
3. 7 天额度卡：已用或剩余百分比、重置时间、倒计时和状态。
4. Provider 支持时显示剩余手动重置次数。
5. Token 用量：输入、输出、Cache Read、Cache Creation 和总 Token。
6. Token 时间范围：今天、7 天、30 天和自定义范围。
7. 最近会话或请求明细，并明确标记来源为 Session 日志或本地代理。

额度窗口是 Provider 返回的固定窗口，不随 Token 时间筛选器改变。界面必须把额度卡与可筛选的 Token 图表分开，避免让用户误以为“今天”会改变 5 小时或 7 天额度。

如果 Provider 不提供某个额度窗口，则显示“此订阅不提供该额度窗口”，不得显示 0%。

### API 模块页面

API 模块与订阅模块同级，集中展示所有启用的 `billingKind = metered` Provider。

页面顶部提供统一时间范围：今天、7 天、30 天和自定义范围。主体展示：

1. API 总览：总 Token、请求数和有可信来源的总费用。
2. Provider 卡片或列表：Azure、OpenRouter 等各自独立显示 Token、请求数、真实费用和费用来源。
3. 模型分布和近期请求明细。
4. 无法取得真实费用时显示“费用不可用”或“估算费用”，不得把未知费用当作零。

API 总览可以求和不同按量 Provider 的费用，但必须清楚标注币种和来源，并保留按 Provider 拆分。订阅额度不得进入 API 总览、费用趋势或请求数。

### 空状态

- 订阅模块没有 Provider：说明“尚未添加此模块的订阅账号”，提供“前往设置添加 Provider”。
- API 模块没有按量 Provider：说明“尚未配置 API Provider”，提供“前往设置添加 API Provider”。
- 所有模块均被隐藏：显示模块管理入口，不渲染空白页面。

## 设置页

设置页新增以下结构：

```text
设置
  用量模块
  Provider
  代理与路由
```

### 用量模块

- 新增订阅模块。
- 修改模块名称、排序和可见性。
- 删除空的自定义订阅模块。
- 查看每个模块当前包含的 Provider 数量。
- API 系统模块不可删除，也不可改成订阅模块。

### Provider

新增或编辑 Provider 时，表单先选择计费方式：

- 订阅：必须选择一个现有订阅模块，也可以在表单内快速新建模块。
- 按量：自动归入 API 模块，不显示无意义的订阅模块选择。

订阅 Provider 表单继续配置额度来源、刷新间隔、Session 来源绑定和所需凭据。按量 Provider 表单继续配置协议、Base URL、凭据与 Token/费用来源。

Provider 编辑必须允许修改显示名称和订阅模块归属。移动 Provider 只改变首页展示位置，不改变其稳定 Provider ID，也不重写历史事件。

### 代理与路由

当前首页中的以下内容全部移入此处：

- 启动或停止本地代理。
- Claude、Codex、Gemini 等协议的静态 RouteBinding。
- 代理监听状态和必要诊断。

文案使用“代理转发目标”而不是只有开发者容易理解的“静态路由”。必须说明该设置决定请求发往哪个按量 Provider，不是用量页面筛选器。

## 数据模型变化

建议新增：

```text
dashboard_modules
  id TEXT PRIMARY KEY
  name TEXT NOT NULL
  kind TEXT NOT NULL CHECK(kind IN ('subscription', 'api'))
  sort_order INTEGER NOT NULL
  visible INTEGER NOT NULL
  is_system INTEGER NOT NULL
  created_at INTEGER NOT NULL
  updated_at INTEGER NOT NULL

usage_providers.dashboard_module_id TEXT NULL
  REFERENCES dashboard_modules(id)
```

约束：

- `billingKind = subscription` 的启用 Provider 必须归属一个 `kind = subscription` 模块。
- `billingKind = metered` 的 Provider 不依赖可编辑模块归属；查询时统一投影到唯一 API 模块。
- 数据库必须保证恰好一个 `kind = api` 的系统模块，并拒绝创建第二个 API 模块。
- 模块删除和 Provider 移动在一个事务中完成。
- 模块名称不是业务键；所有关联使用稳定模块 ID。

不在 UsageEvent 上新增可变展示模块字段。历史事件继续以 `providerId` 和 `productGroupId` 保持不可变归因。

## API 与状态边界

前端至少需要以下模块配置能力：

```text
list_dashboard_modules
save_dashboard_module
reorder_dashboard_modules
set_dashboard_module_visibility
delete_dashboard_module
```

Provider 保存接口增加可选的 `dashboardModuleId`，并在订阅 Provider 启用时校验其归属。现有 Dashboard 查询可以继续返回 Provider 和产品聚合，但前端或后端投影层必须同时提供稳定模块 ID，不能依赖显示名称匹配。

模块导航状态属于本地界面偏好；上次选中模块可存入应用设置，不属于 UsageEvent 或 Provider 账单数据。

## 错误处理与降级

- 单个订阅 Provider 的额度刷新失败时，保留最后成功额度并显示“数据已过期/刷新失败”；其他 Provider 和模块保持可用。
- 单个 API Provider 查询失败时，API 总览标记为部分数据，不得把缺失 Provider 当作零费用。
- 模块配置读取失败时显示一条可恢复错误，不回退到写死的模块名单。
- 删除或隐藏当前模块后，导航立即选择可用回退模块。
- 浏览器中单独运行 Vite renderer 时，不应连续显示多个 `window.__TAURI_INTERNALS__` 技术错误；显示一个明确的预览提示：“当前为界面预览，需在 LLM Usage Bar 桌面应用中读取本地用量。”

## 无障碍与交互要求

- 顶层模块切换器使用单选 Tab 语义，支持方向键切换、清晰焦点和 `aria-selected`。
- 二级 Provider 实例切换器同样具有可识别的选中状态，但与顶层模块使用不同的可访问名称。
- 额度不能只靠颜色表达；同时显示数值、标签和状态文字。
- 重置时间同时提供绝对时间和易读倒计时。
- 横向溢出的模块导航可通过键盘访问，不隐藏当前焦点。
- 设置中的删除、移动和隐藏操作必须说明影响范围，并在不可逆或会导致模块为空时确认。

## 测试策略

### 数据与迁移

- 新数据库生成四个默认模块，但前端不依赖固定名称分支。
- 现有订阅 Provider 可可靠映射时进入对应模块，无法映射时进入按需创建的“其他订阅”模块并标记为待归类。
- 所有按量 Provider 出现在唯一 API 模块。
- 模块改名、排序、隐藏和 Provider 移动不改变历史 UsageEvent。
- 删除包含 Provider 的模块被拒绝，或在同一事务中完成明确迁移。

### 前端

- 顶部模块按后端返回顺序渲染；新增第五个订阅模块无需修改组件即可出现。
- 点击模块只改变可见页面，不调用 Provider 切换、路由修改或实时配置写入。
- Codex 模块能在 GPT Plus（个人）和 GPT Pro（工作）之间切换，额度和 Token 不混合。
- API 模块同时展示 Azure 和 OpenRouter，订阅额度不会出现。
- 静态路由与 Provider 配置不再出现在首页，但在设置中可完整访问。
- 单独浏览器预览只显示一条友好提示，不显示 Tauri 内部错误堆栈。
- Tab 键、方向键和屏幕阅读器语义覆盖两级切换器。

### 回归

- Provider、QuotaSnapshot、UsageEvent 和 RouteBinding 现有可信边界保持不变。
- 订阅额度不会进入 Token 或费用聚合。
- API 缺失费用不会变成零。
- 原 CC Switch 数据目录和实时配置隔离规则保持不变。

## 验收标准

1. 首次使用默认显示 Codex、Claude Code、Kimi Coding Plan、API 四个模块；它们来自数据库配置而不是前端常量。
2. 用户可在设置中新增一个 Gemini 模块并添加订阅 Provider，返回首页后无需重启或更新代码即可看到新模块。
3. GPT Plus 与 GPT Pro 作为 Codex 模块内的两个 Provider 实例，分别显示自己的 5 小时、7 天额度和 Token。
4. Azure 与 OpenRouter 只出现在 API 模块，按统一时间范围展示，同时保留逐 Provider 拆分。
5. 首页不再显示 Provider 配置、静态路由和代理启动卡；这些能力完整迁移到设置。
6. 顶部模块切换不会修改任何客户端当前 Provider、RouteBinding 或代理目标。
7. 模块改名、排序、隐藏或移动 Provider 不会改写历史用量事件和账单身份。
8. 额度失败、API 部分失败和非 Tauri 预览都有单一、明确、可恢复的错误状态。

## 后续实施边界

实施应拆为三个可独立验证的阶段：

1. 数据层：模块表、Provider 归属、默认数据、迁移与模块 CRUD。
2. 设置层：模块管理、Provider 归属和代理/路由配置搬迁。
3. 首页层：动态顶层导航、订阅 Provider 二级切换、统一 API 页面和预览降级。

每个阶段必须保留现有 Provider、事件、额度和路由行为，并在完成后运行对应迁移、聚合、Tauri command、前端交互和完整回归测试。
