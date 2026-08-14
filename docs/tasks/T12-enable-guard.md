# T12:写指针之前先确认 router 真的在监听

> **先读 [`README.md`](README.md) 的铁律。** 依赖:T10、T11 已合并。
> 可与 T13 并行 —— 你只碰 `server.rs` 顶部(`start` 附近)和 `commands/router.rs`,
> T13 在 `server.rs` 的转发/流式段(456 行之后)。**互相不要越界。**

---

## 目标

T10 交付后有一个会把用户卡死的组合:

```
端口 8788 被占 → start() 绑定失败,只记 log::error!,router 没起来
用户点「启用」 → enable_router_pointer 照样把指针写进 config.toml
用户重启 Codex → 每个请求都连接被拒
```

**而指针按设计决定 34 是不能再改回去的** —— 用户只剩 `scripts/codex-unroute.sh`
这条逃生路。这不是理论问题:8788 被占最常见的原因就是上一次 app 没退干净。

**写指针之前先确认 router 在监听。没在监听就拒绝写,并告诉用户为什么。**

---

## 1. 让 `start()` 公布自己绑到了哪个端口

`src-tauri/src/router/server.rs`,**只改 `start` / `bind_loopback` 那一段
(第 60–110 行附近)**,不要碰这个文件的其他任何地方。

```rust
/// 已经成功绑定的端口。`start()` 绑定成功后写入，失败时保持 None。
///
/// 用 AtomicU16 而不是 Mutex：这是一个只写一次、之后频繁读的标量，
/// 而且读的一方（tauri 命令）不该有任何机会阻塞在 router 的锁上。
static LISTENING_PORT: AtomicU16 = AtomicU16::new(0);

/// router 当前监听的端口；`None` 表示没起来。
///
/// 0 是「未绑定」的哨兵值——端口 0 在 bind 语义里是「随便给一个」，
/// 而我们永远显式指定端口，所以它不会是一个真实的监听端口。
pub fn listening_port() -> Option<u16> {
    match LISTENING_PORT.load(Ordering::Relaxed) {
        0 => None,
        port => Some(port),
    }
}
```

在 `start()` 里**绑定成功之后**、`axum::serve` 之前写入。
`bind_loopback` 返回 `Err` 时**不要写**。

### 1.1 服务退出时要清掉

`axum::serve(...).await` 返回(不管正常还是出错)之后,把 `LISTENING_PORT`
置回 `0` —— 否则 router 挂掉之后 `listening_port()` 还在说「我活着」,
这个守卫就白做了。

---

## 2. `enable_router_pointer` 加守卫

`src-tauri/src/commands/router.rs`,只改这一个命令。

```
1. port = read_router_port(db)
2. match router::server::listening_port()
     None            → 返回 Err，告诉用户 router 没起来（见 §2.1 的措辞）
     Some(p) if p != port → 返回 Err（配置里的端口和实际监听的不一致，
                            通常是改了 router.port 但没重启 app）
     Some(_)         → 继续，写指针
3. 其余照旧（写指针 → 清缺口标记）
```

### 2.1 错误信息要能让用户自己解决

**不要返回 `"router not listening"` 这种。** 用户看到之后要知道下一步做什么。
两种情况分别给:

- 没监听:说明 router 没能在端口 N 上起来,最常见的原因是端口被占;
  建议换一个端口或重启 app
- 端口不一致:说明配置里是 N、实际监听的是 M,改过端口需要重启 app

**措辞你自己写**,但必须包含具体端口号,并且指出一个可执行的下一步。

---

## 3. 测试

`listening_port()` 是进程级全局状态,**测试之间会互相污染**。所以:

1. **不要**写「起一个真 server 再读 `listening_port()`」这种测试 —— 它会和
   同进程里其他测试打架
2. 把守卫的判定抽成一个**纯函数**,测它:

```rust
/// 只做判定，不读全局状态。`listening` 由调用方传入。
fn pointer_enable_guard(configured_port: u16, listening: Option<u16>) -> Result<(), String>;
```

必测:

| # | 场景 | 期望 |
| --- | --- | --- |
| 1 | `listening = None` | `Err`,信息里含配置的端口号 |
| 2 | `listening = Some(8788)`,配置 8788 | `Ok` |
| 3 | `listening = Some(9000)`,配置 8788 | `Err`,信息里**两个端口都出现** |

第 3 条要两个端口都出现,是因为用户得看出「配的是这个、实际是那个」。

---

## 4. 明确不要做的事

- ❌ **不要碰 `server.rs` 第 110 行之后的任何代码** —— 那是转发与流式路径,
  T13 正在那里工作
- ❌ 不要碰 `router/pointer.rs`、`router/auth.rs`
- ❌ 不要改指针「只写一次」的语义(决定 34)
- ❌ 不要在启动时自动写指针
- ❌ 不要为了「更保险」去真的发一个 HTTP 请求探活 —— 那要处理超时、
  要造一个不产生副作用的探测端点,成本远高于收益
- ❌ 不要加新依赖

---

## 5. 完成的标准

- `listening_port()` 存在;绑定成功写入,`axum::serve` 返回后清零,绑定失败不写
- `enable_router_pointer` 在没监听 / 端口不一致时拒绝写指针
- 两种错误信息都含具体端口号和一个可执行的下一步
- 三条测试通过(判定是纯函数,不依赖全局状态)
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings`**
