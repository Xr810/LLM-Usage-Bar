# T17:按职责重排目录,让 `services/` 这个名字消失

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T14、T15、T16 全部合并**。
> **必须单独做** —— 它动整个 `src-tauri/src/`,和任何并行任务都会冲突。
>
> **这是纯搬运。一行逻辑都不许改。** 它的价值全在「改完之后目录名能表达职责」,
> 一旦掺进逻辑改动,出岔子时就没法归因了(2026-08-14 已有教训)。

---

## 目标

模块化设计 §9.1 的问题陈述:

> 最能说明问题的是 `services/`:21,650 行,而这个名字不表达任何边界——
> 什么都能塞,事实上也确实什么都塞了。**一个目录叫「服务」等于没分类。**

§9.2 的目标划分:

```
                    api
                     ↓
        route  ·  quota  ·  ingest
                     ↓
                   store
                     ↓
                   model
```

| 新模块 | 从哪来 |
| --- | --- |
| `model` | `usage/domain.rs`、`services/subscription/mod.rs` 里的共享类型 |
| `store` | `database/` 整个 |
| `ingest` | `services/ingest/` 整个 + `services/session_usage*.rs` 那几个转发层 |
| `quota` | `usage/quota.rs` + `claude_quota.rs`(2,842 行,**现在躺在根目录**) |
| `route` | `router/` 整个 |
| `api` | `api/`(T16 建的)+ `commands/` |
| `config` | `settings.rs`、`app_config.rs`、`provider_defaults.rs` |
| `secrets` | `credentials/` 整个 |

---

## 1. 铁律:只搬不改

**允许的改动只有三种:**

1. 文件位置(`git mv`)
2. `mod` / `use` 路径
3. 为保持可见性必需的 `pub use` 转出

**任何函数体、任何结构体字段、任何字符串字面量、任何测试断言 —— 一个字符都不许动。**

验收方式:

```bash
git diff main...HEAD -- '*.rs' | grep '^[+-]' | grep -v '^[+-][+-]' \
  | grep -vE '^[+-]\s*(pub )?(use|mod|pub mod|pub\(crate\) (use|mod))' \
  | grep -vE '^[+-]\s*$'
```

**这条命令的输出应当为空。** 不为空的每一行都要在报告里逐条解释为什么必须改。

---

## 2. 分批做,每批单独提交

`services/` 有 31 个文件、`usage/` 有 32 个。**一次全搬会做成一个没法复核的巨型 diff。**
按这个顺序,**每批一个提交,每批之后跑一次六项检查**:

| 批次 | 内容 |
| --- | --- |
| 1 | `database/` → `store/` |
| 2 | `credentials/` → `secrets/` |
| 3 | `router/` → `route/` |
| 4 | `services/ingest/` + `session_usage*.rs` → `ingest/` |
| 5 | `usage/quota.rs` + `claude_quota.rs` → `quota/` |
| 6 | `settings.rs` + `app_config.rs` + `provider_defaults.rs` → `config/` |
| 7 | 共享类型 → `model/` |
| 8 | `commands/` + `api/` → `api/`;**`services/` 剩下的文件按职责归位,直到这个目录能被删掉** |

**第 8 批最难。** `services/` 里还有 Coding Plan、WebDAV 同步、Claude CLI 认证
这些东西 —— 它们不属于上面任何一个模块。**遇到归不了位的,停下来在报告里列出来,
不要硬塞。** 宁可 `services/` 少了一半也别硬塞出一个错误的归类。

---

## 3. 依赖方向必须单向(§9.3 第一条)

```
model ← store ← {ingest, quota, route} ← api
```

`model` 谁都能依赖,它谁都不依赖。搬完之后自查:

```bash
grep -rn "use crate::" src-tauri/src/model/
```

**输出里不许出现 `store` / `ingest` / `quota` / `route` / `api`。**
若出现,说明有类型放错了层,报告里说明并把它挪出 `model`。

---

## 4. 明确不要做的事

- ❌ **不许改任何逻辑** —— 这是本任务唯一的红线
- ❌ 不要顺手重命名函数或类型
- ❌ 不要合并或拆分文件(只挪位置)
- ❌ 不要为了「更整齐」去调整 `pub` 可见性,除非编译器要求
- ❌ 不要碰 `src/`(前端)、不要加新依赖

---

## 5. 完成的标准

- 八批各自一个提交,每批之后六项检查全绿
- §1 那条 diff 命令输出为空(或每一行都有解释)
- §3 那条依赖自查输出干净
- **`services/` 目录不复存在**,或报告里列出「归不了位、有意留下」的文件及理由
- 全部测试通过且**断言一个字未改**
- 最终 `clippy --all-targets -- -D warnings` 绿
- 报告里给出:重排前后各目录的文件数与行数对照表
