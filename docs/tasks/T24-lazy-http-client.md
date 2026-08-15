# T24:`http_client::init` 别再堵在启动主线程上

> **先读 [`README.md`](README.md) 的铁律。** 依赖:T22 已合并。
> **与 T25 都改 `lib.rs`**(不同区段,冲突是一行级的),可并行但合并时留意。
>
> **这个任务碰启动流程。** 改坏了 app 起不来,而单元测试不一定发现。

---

## 0. 数字(T22 实测,dev profile)

| 启动步骤 | 实测 |
| --- | --- |
| `http_client::init(None)` **首次** | **119–167 ms**(主线程同步) |
| 其后再调 | ~0.3–1 ms |
| DB prepare + 预检 + 打开 + 建 schema(老用户) | 25.4 ms |
| DB(全新安装) | 75.3 ms |

**`http_client::init` 是启动路径上最大的一块**,而且它比开数据库还慢一倍以上。
那 120–170 ms 花在 TLS 后端初始化和连接池搭建上 —— **而 app 刚起来时未必要发请求。**

---

## 1. 现状

```
src-tauri/src/lib.rs:1058   crate::http_client::init(proxy_url.as_deref())
src-tauri/src/lib.rs:1076   crate::http_client::init(None)          ← 代理失败时的回落
src-tauri/src/services/s3.rs:885,895                                 ← 测试里,不要动
```

`http_client` 用的是 `OnceLock` 式全局(`GLOBAL_CLIENT`),`get()` 里已经有
「没初始化就 fallback 建一个」的分支(`http_client.rs:209` 附近,
带 `[GP-004] Client not initialized, using fallback` 的告警)。

---

## 2. 改法:惰性化,但保住代理配置

**不能简单地「删掉启动时的 init,靠 get() 的 fallback」** —— 那个 fallback 建的是
**不带代理配置**的客户端,用户配了代理就会被绕过,而且只打一条 warn。

要做的是:

1. **把启动时的 `init` 从主线程挪走**,不要让它堵住 setup ——
   放进 `tauri::async_runtime::spawn`,或者改成第一次 `get()` 时按已保存的代理配置
   惰性初始化(二选一,**在报告里说明你选了哪个、为什么**)
2. **保证代理配置不丢**:无论怎么改,用户配了代理时第一个真实请求**必须**走代理
3. **保证没有竞态**:初始化在飞的时候如果有人调 `get()`,不能拿到一个没代理的客户端
   然后把它缓存下来

**第 3 条是这个任务真正的难点。** 想清楚再动手,想不清楚就停下来报告。

---

## 3. 硬要求

- ✅ 用户配了代理 → 第一个真实 HTTP 请求走代理(**要有测试**)
- ✅ 代理配置失败 → 仍然回落到直连,行为与现在一致
- ✅ `update_proxy` 的热更新路径不受影响
- ✅ 启动时 `router::server::start()` 仍在早期调用(决定 5②)——
  **不要为了挪 http_client 把 router 的启动顺序也动了**

---

## 4. 测试

1. 配了代理时,`get()` 返回的客户端带代理(用 `get_current_proxy_url()` 或等价方式验证)
2. 代理配置非法时回落直连,且不 panic
3. 并发场景:多个线程同时 `get()`,都拿到同一个正确配置的客户端
   (第 3 条是 §2 那个难点的证明)

---

## 5. 明确不要做的事

- ❌ 不要动 `services/s3.rs` 里那两处(测试代码)
- ❌ 不要改 `router::server::start()` 的调用时机
- ❌ 不要顺手改 `get()` 之外的 http_client 公开 API
- ❌ 不要碰 `src/`、不要加新依赖

---

## 6. 完成的标准

- 启动主线程上不再有那 120–170 ms
- 三条测试通过,尤其第 3 条(竞态)
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings`**
- 报告里给出**改动前后的启动耗时对照**(用与 T22 同样的量法),
  以及你选了「后台 spawn」还是「惰性初始化」、为什么
