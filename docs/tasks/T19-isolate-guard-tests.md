# T19:让守卫测试不再快照真实的 `~/.cc-switch`

> **先读 [`README.md`](README.md) 的铁律。** 依赖:无。可与 T16、T17 之外的任何任务并行
> (你只碰 `database/identity_migration.rs` 与 `config.rs` 的一行)。

---

## 0. 背景:T18 已经把根因查实了

`database::identity_migration::tests` 里有**三个**测试对**真实的 `~/.cc-switch`**
做前后快照,断言逐字节一致:

```
lexical_alias_to_real_legacy_directory_is_rejected_before_any_write        (2964)
symlink_alias_to_real_legacy_directory_is_rejected_before_any_write       (2981)
hardlink_old_path_alias_to_real_legacy_database_is_rejected_before_any_write (2998)
```

它们共用助手 `assert_guard_rejects_without_changing_real_legacy`(2331 行),
断言信息是 `guard call changed protected ~/.cc-switch entries or file bytes`。

**T18 的实测结论**(2026-08-15,证据完整):开发机上运行着旧版
`CC Switch.app`,每分钟往 `~/.cc-switch/logs/cc-switch.log` 追加日志;
两次快照之间夹住它一次写入,断言就假阳性。复现率约 1/8 到 1/20。
**这解释了 2026-08-14 起累计 5 次「跑一遍红、重跑就绿」。**

`#[serial_test::serial]` 只在测试进程内互斥,**挡不住进程外的写入**。

---

## 1. 关键事实:只改测试是改不动的

```rust
// identity_migration.rs:503 —— 生产代码
let home = dirs::home_dir().ok_or_else(|| { … })?;
reject_legacy_namespace_with_protected(app_dir, &home.join(LEGACY_DATA_DIR))
```

守卫**写死** `dirs::home_dir()`,不认 `LLM_USAGE_BAR_TEST_HOME`。
所以测试无法把受保护目录重定向到临时目录。

### 1.1 顺带修掉一个真实的不一致(这是本任务动生产代码的理由)

`crate::config::get_home_dir()`(`config.rs:15`)会优先读
`LLM_USAGE_BAR_TEST_HOME` / `CC_SWITCH_TEST_HOME`。

**今天的行为是不一致的**:带着 `LLM_USAGE_BAR_TEST_HOME` 跑时,app 的数据目录
搬到了假 home,**但这个守卫仍然在保护真实的 `~/.cc-switch`** ——
它保护的是一个与当前 home 无关的目录。

**改成 `get_home_dir()` 之后两者才一致。** 这不是为了让测试好写而放宽保护,
是把保护对准当前真正在用的 home。

---

## 2. 要做的两件事

### 2.1 生产代码:一行

`identity_migration.rs:503` 的 `dirs::home_dir()` 改成
`crate::config::get_home_dir()`(注意它返回 `PathBuf` 不是 `Option`,
错误分支要跟着调整)。

**只改这一处。** 文件里其他 `dirs::home_dir()` 调用(如果有)先别动,
在报告里列出来由我判断。

### 2.2 三个测试:在隔离的假 home 里造一个假 legacy 目录

```
1. tempfile::tempdir() 造一个假 home
2. 在里面建 <fake_home>/.cc-switch/,放几个文件(至少要有 LEGACY_DATABASE_FILE,
   hardlink 那个测试需要它)
3. 设 LLM_USAGE_BAR_TEST_HOME 指向假 home
4. 照原样跑守卫,照原样做前后快照断言
5. 用完清掉环境变量
```

**断言逻辑一个字不要改** —— 快照比对是这三个测试的价值所在,要保留。
变的只是「被保护的那个目录是谁」。

`real_legacy_directory()` 助手改成返回假 home 里那个目录;
「目录不存在就 skip」的分支可以去掉了(现在总是存在,因为是你自己造的)。

---

## 3. 硬要求

- ✅ **三个测试仍然在断言「守卫被拒绝 + 目录一字节未变」** ——
  不许降级成「只断言返回了 Err」
- ✅ **不许加 `#[ignore]`**,也不许指望 `#[serial]` 解决 ——
  那是把问题藏起来
- ✅ 环境变量要**用完就清**,否则会污染同进程里其他测试
  (`config.rs:390` 附近有现成的设置/清除写法,照抄)
- ✅ 现有其他测试全部继续通过,断言不改

---

## 4. 顺带排查(报告即可,不要动手)

T18 提到新版 `LLM Usage` 在写 `~/.llm-usage-bar`。**排查一遍**:
仓库里还有没有别的测试对**真实 home 下的目录**做快照或断言?

```bash
grep -rn "dirs::home_dir" src-tauri/src
```

逐处判断是「生产代码合理使用」还是「测试在碰真实 home」,
**列出来,不要改** —— 换个目录踩同一个坑不值得再来一次。

---

## 5. 验收

- 三个测试改完之后,**连跑 30 轮全量 `cargo test` 不出现那个失败**
  (T18 观察到的复现率是 1/8 到 1/20,30 轮足够)
- 报告里贴 30 轮的实际结果(通过/失败计数)
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings`**
- §4 的排查清单

**如果 30 轮里还出现别的偶发失败**,那是另一个问题:记下名字和完整输出,
不要顺手修 —— 报告给我。
