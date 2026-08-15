# T20:把扩展缝焊住 —— 依赖方向守卫

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T17 已合并**(2026-08-15)。
> **可与任何任务并行** —— 它只新建一个测试文件。
>
> **未排期,尚未派出。**
>
> **这不是重构任务。** 它一行业务逻辑都不改,产出是「以后想加一整块新功能时
> 不用动 core」这个属性的**保险**,不是那个属性本身。

---

## 0. 为什么做这个,而不是直接做扩展

2026-08-15 讨论的结论:

> 「以后加一整块新功能会不会要大改 core」—— **代码层面基本不成立**。
> 目录分层是 `git mv`,命令注册是只加不改的追加表,前端导航是 158 行的 `App.tsx`。
> 这几样的成本跟时间无关,提前做不省任何事。
>
> **会随时间变贵的只有两样**:一是 `store/schema.rs`(数据落到用户机器上就撤不回来),
> 二是**已划好的线被慢慢磨平**。

第一样已由 [`README.md`](README.md) 铁律第 9 条堵住(2026-08-15 落地)。
**本任务负责第二样。**

真正的分层(`sync/` 与 `providers/`)见
[`T21-provider-modules-and-sync.md`](T21-provider-modules-and-sync.md);
形态见 [`../design/2026-08-15-provider-modules-and-feature-domains.md`](../design/2026-08-15-provider-modules-and-feature-domains.md)。
**本任务与 T21 谁先做都行**,但先做本任务能让 T21 的搬运结果被自动验证。

---

## 1. 要断言什么

[模块化设计](../design/2026-08-14-modular-core-and-providers.md) §9.3 第一条:

```
model ← store ← {ingest, quota, route} ← api
```

T17 已经把目录搬成这个形状,但**没有任何东西阻止三个月后有人写一行
`use crate::api::...` 到 `model/` 里**。守卫就是那个阻止者。

### 1.1 分层表(守卫的核心,写成代码里的一张常量表)

设计文档决定把「谁是地基、谁是可拆的功能域」这个信息**交给守卫承载**,
而不是用 `features/` 之类的目录前缀 —— 目录名是装饰,守卫是机器检查的。
所以这张表就是那份信息的唯一权威处:

| 层 | 模块 | 允许依赖 |
| --- | --- | --- |
| 地基 0 | `model` | **什么都不许** |
| 地基 1 | `store` `config` `secrets` | `model` + 工具(`error`/`http_client`/`product_identity`) |
| 功能域 | `usage` `ingest` `quota` `route` | 地基 + 同域内部 |
| 功能域 | `sync`(T21 后存在) | 地基 **+ 不得依赖任何其他功能域** |
| provider | `providers`(T21 后存在) | 地基 + `quota` 的接口定义 |
| 顶 | `api` | 全部 |

**写成 `const` 表驱动,不要写成一串 if。** T21 落地后加 `sync` / `providers`
就是加两行,这正是这张表的意义。

### 1.2 本次必须真正打开的两条

`sync/` 和 `providers/` 还不存在(T21 未做),**表里给它们留行但跳过扫描**。
本次真正生效的是:

| # | 断言 |
| --- | --- |
| A | `model/` 下任何 `.rs` 不得 `use` 除工具模块外的任何 crate 内模块 |
| B | `store/` 下任何 `.rs` 不得 `use crate::{ingest,quota,route,api,usage}` |

### 1.3 不要做的

**不要**把设计 §4 那条「core 里不许出现 provider 名字」也做进来。
那是另一个任务的地盘,而且现在违规量很大(实测:`store` 24 个文件里 14 个提到厂商名、
`usage` 35 个里 27 个),混进来会让本守卫一上来就是一张巨型 allowlist,失去意义。

---

## 2. 怎么写:抄现成的

**仓库已经有一套一模一样性质的守卫**:`tests/config/productIdentity.test.ts`
—— 扫全仓 + allowlist 棘轮。关键是它的 allowlist 结构
(`tests/config/productIdentityAllowlist.ts`):**已知违规逐条列出,并断言实际违规
集合与 allowlist 完全相等** —— 新增违规会失败,修好了不从 allowlist 删掉也会失败。
**这个双向断言是重点**,只做「不许新增」的守卫会烂掉。

放在哪:**Rust 侧**,`src-tauri/tests/` 下新建一个集成测试。被守的是 Rust 模块边界,
让它跟着 `pnpm rust -- test` 跑,不要绕到前端测试里去读 Rust 源码。

用 `env!("CARGO_MANIFEST_DIR")` 定位 `src/`,标准库 `std::fs` 递归读目录即可 ——
**不要为此加 `walkdir` 之类的新依赖**(铁律 2)。

### 2.1 允许的宽松

- `use crate::` 开头的行和 `crate::x::y` 这类内联路径,**两种都要抓**
- `#[cfg(test)]` 块内的 `use` **同样受约束**(测试里跨层引用同样是信号)
- 注释和字符串里的匹配可以不管 —— 误报代价低于漏报,但实现上容易排除就排除

---

## 3. 交付时 allowlist 里必然有东西,这是正常的

**已知的违规(2026-08-15 实测,必须出现在 allowlist 里):**

```rust
// src-tauri/src/model/domain.rs:4  —— 违反断言 A
use crate::usage::status::{PaceBasis, SourceClassification, UsageStatus};
```

成因见 [HANDOFF §15.1](../../HANDOFF.md):那三个纯枚举住在 `usage/status.rs`
(1,262 行,还依赖 `rhythm`),把它们并进 `model` 需要**拆文件**,而 T17 的红线是
「只搬不改」。**消除它是另一个任务。**

`store/` 那条实测有多少违规**未预先统计,执行时如实列出**。

> **红线:不要为了让 allowlist 空掉而去改业务代码。** 如实列进去,并在报告里逐条
> 说明是什么类型、为什么在那儿 —— 那份清单本身就是下一个任务的输入。
> 硬改代码去凑一个空 allowlist 是本任务唯一的红线。

---

## 4. 明确不要做的事

- ❌ 不要动 `store/schema.rs`,不要改 `SCHEMA_VERSION`,不要重命名任何表
- ❌ 不要建 `sync/` / `providers/`,不要挪任何模块(那是 T21)
- ❌ 不要碰 `src/`(前端)
- ❌ 不要为了凑空 allowlist 去改业务代码(§3)
- ❌ 不要把「core 里不许出现 provider 名字」做进来(§1.3)
- ❌ 不要加新 crate 依赖

---

## 5. 完成的标准

- `src-tauri/tests/` 下新增一个守卫测试,**表驱动**(§1.1),含 allowlist,**双向断言**
- 表里为 `sync` / `providers` 留了行并注明「T21 后启用」
- 故意在 `model/` 里加一行 `use crate::api::...` 能让该测试**失败**;删掉后恢复通过
  —— **报告里贴这次实验的实际输出**,不要只说"验证过了"
- 六项检查全绿
- 报告里给出:allowlist 的完整内容,以及每一条的类型与成因

---

## 6. 这个任务不解决什么(诚实说明)

它**不会**让「加一整块新功能」变得更容易 —— 那需要 T21 的分层、迁移链拆版本轴、
前端顶层导航。

它只做一件事:**让已划好的线不被磨平。** 没有守卫,T17 那八批搬运的成果会被慢慢
磨平,**到那时候才是真的贵**。
