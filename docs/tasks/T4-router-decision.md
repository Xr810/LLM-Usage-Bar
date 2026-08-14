# T4:路由决策(纯函数,无 IO)

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T3 必须已完成并合并**。
> 可与 T5 并行。

---

## 目标

写一个**纯函数**:给定「这个模型的候选列表」+「当前的拉黑状态」+「当前模式」,
算出**应该按什么顺序去试哪几家**。

**这个任务不碰数据库、不发网络请求、不读文件。** 全部输入由调用方喂进来,
输出是一个列表。因此它 100% 可单元测试,也必须被测透。

---

## 1. 新建文件

`src-tauri/src/router/mod.rs`(新建 `router` 目录)
`src-tauri/src/router/decision.rs`

在 `src-tauri/src/lib.rs` 里加 `mod router;`(找到其他 `mod xxx;` 的地方,加一行)。

---

## 2. 类型与签名(照写,不要改)

```rust
use crate::database::dao::router::ModelRoute;

/// 路由模式。自动:按顺序试、失败换下一家。手动:只用指定那家,不换。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteMode {
    Auto,
    Manual { provider_id: String },
}

/// 一条被拉黑的记录:某个 (provider, 模型) 在 until_ms 之前不再尝试。
/// provider 级别的拉黑用 logical_model = None 表示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blacklisted {
    pub provider_id: String,
    pub logical_model: Option<String>,
    pub until_ms: i64,
}

/// 一个候选:去哪家、把模型名换成什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub provider_id: String,
    pub upstream_model: String,
    /// 仅用于 UI 展示：这一家当前是否处于拉黑期。
    /// 手动模式下会出现 true（手动不受拉黑约束，但要让用户看见）。
    pub blacklisted: bool,
}

/// 给定模型的候选队列。**注意返回的是队列,不是"当前的 provider"**。
///
/// 这个签名是硬约束(设计文档决定 23):v1 虽然只有全局顺序,但签名必须以模型为
/// 入口。写成 `fn current_provider() -> Provider` 的话,到 v2 支持"每个模型不同
/// 顺序"时每个调用点都要改。
///
/// - `routes`:该模型在各家的映射,**调用方保证已按 priority 升序**
/// - `blacklist`:当前所有拉黑记录(不必预筛)
/// - `now_ms`:当前时刻,由调用方传入(便于测试)
pub fn candidates_for(
    routes: &[ModelRoute],
    blacklist: &[Blacklisted],
    mode: &RouteMode,
    now_ms: i64,
) -> Vec<Candidate>;
```

---

## 3. 行为规格(逐条实现,逐条测试)

### 自动模式 `RouteMode::Auto`

1. 按 `routes` 给定的顺序遍历
2. **跳过**正在拉黑期内的(`until_ms > now_ms`)。判定要同时考虑两种拉黑:
   - 精确到行的:`provider_id` 且 `logical_model == Some(该模型)`
   - 整个 provider 的:`provider_id` 且 `logical_model == None`
3. 已过期的拉黑(`until_ms <= now_ms`)**视为不存在**
4. 返回剩下的全部候选,`blacklisted` 一律为 `false`
5. 如果全部被拉黑,**返回空 Vec**(不是错误,由调用方决定怎么办)

### 手动模式 `RouteMode::Manual { provider_id }`

1. 只返回 `provider_id` 匹配的那**一个**候选(最多一个)
2. **拉黑不生效** —— 即使它在拉黑期内也要返回(设计文档决定 29:手动模式下拉黑只显示不生效)
3. 但要把 `blacklisted` 字段设成真实状态,好让 UI 显示「暂不可用」
4. 如果该 provider 在这个模型上根本没有映射,**返回空 Vec**

---

## 4. 测试(必须写,而且必须过)

在 `decision.rs` 末尾加 `#[cfg(test)] mod tests`。**至少覆盖下面每一条**,
一条一个测试函数,函数名要说人话:

| # | 场景 | 期望 |
| --- | --- | --- |
| 1 | 自动,无拉黑 | 原样返回全部,顺序不变 |
| 2 | 自动,第一家被行级拉黑 | 跳过它,第二家排第一 |
| 3 | 自动,第一家被 provider 级拉黑(`logical_model = None`) | 同样跳过 |
| 4 | 自动,拉黑已过期(`until_ms == now_ms`) | **不跳过**(边界:等于视为已过期) |
| 5 | 自动,拉黑还差 1 毫秒到期(`until_ms == now_ms + 1`) | 跳过 |
| 6 | 自动,全部被拉黑 | 返回空 Vec |
| 7 | 自动,`routes` 为空 | 返回空 Vec |
| 8 | 手动,目标未被拉黑 | 只返回它一个,`blacklisted == false` |
| 9 | 手动,**目标正在拉黑期** | **仍然返回它**,且 `blacklisted == true` |
| 10 | 手动,目标在该模型上没有映射 | 返回空 Vec |
| 11 | 自动,某家对**别的模型**被拉黑 | **不受影响**,照常返回 |

第 11 条特别重要:拉黑的单位是 (provider × 模型),官方在 `gpt-5.6-sol` 上被拉黑
不该影响它在 `gpt-5.5` 上的可用性(设计文档决定 13)。

---

## 5. 明确不要做的事

- ❌ **不要在这个文件里读数据库**。所有输入都是参数
- ❌ 不要发网络请求
- ❌ 不要读系统时间(`now_ms` 是参数,测试要靠它)
- ❌ 不要实现「失败了该不该换下一家」的判断 —— 那是 T5
- ❌ 不要实现拉黑的**写入**(什么时候拉黑、拉多久)—— 那是 T6 的事,本任务只**读**
- ❌ 不要加新依赖

---

## 6. 完成的标准

- 两个新文件建好,`mod router;` 已挂上
- `candidates_for` 签名与本文完全一致
- 11 条测试全部通过
- 六项检查全绿,`test result:` 行贴进报告
