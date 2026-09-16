<div align="center">

# LLM Usage Bar

### 你的 AI 编程工具到底花了多少 —— 订阅额度和 API 花费，都在菜单栏里

[![Platform](https://img.shields.io/badge/platform-macOS%2012%2B-lightgrey.svg)](#安装)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

[English](README.md) | 中文 | [日本語](README_JA.md) | [Deutsch](README_DE.md) | [更新日志](CHANGELOG.md)

*Fork 自 [CC Switch](https://github.com/farion1231/cc-switch)，围绕用量追踪重建 —— 见 [致谢](#致谢)。*

</div>

## 它做什么

同时用 Claude Code 和 Codex 的话，用量是散在几个永远对不上的地方的：Claude 订阅有五小时窗口和每周窗口，Codex 有自己的 OAuth 额度，另外还有一堆按 token 计费的 API key。每个都有自己的页面、自己的重置时钟，没有一个共同的总数。

LLM Usage Bar 在本地把这些全读出来，在菜单栏给出一个答案：**还剩多少，以及你消耗得有多快。**

它不代理你的请求，不管理你的 CLI 配置，也不需要注册账号。它读的是你的工具本来就写在磁盘上的会话日志和额度文件，以及用你自己提供的 key 去调供应商的账单接口。

## 它跟踪两类东西

**订阅额度** —— 你已经付过钱的那些按百分比计的窗口。

- **Claude** —— 五小时和每周两个窗口，来自 Claude Desktop 的本地套餐历史和 Claude Code 的状态栏桥接。重置时刻会被锁存，因此不管哪个数据源最后刷新它都不会丢，并且始终绑定在提供它的那个账号上。
- **Codex** —— 用你已有的 OAuth 会话查询额度。
- **Coding Plan** —— Kimi For Coding、智谱 GLM（个人版与团队版）、MiniMax、火山方舟。带用量重置次数的套餐会显示还剩几次以及各自的过期时间。

**API 花费** —— 真金白银，按 key 算。

- 给一个 Provider 配置一列**具名 API key**，每把 key 各自报告自己的日花费、月花费、剩余预算，以及数字是什么时候抓取的。有两把以上 key 的 Provider 还会显示合计。
- 花费来自供应商自己的账单接口。目前接好的是 OpenRouter；其他预设需要先接上各自的接口才会有数字。
- 可以给单个 Provider 设日预算，也可以设一个总的 API 预算，超速了菜单栏会告诉你。

## 用量红绿灯

菜单栏只给一个指示，而不是丢给你一个要自己解读的数字。它把当前的燃烧速度投影到重置时钟剩下的时间上：

- **健康** —— 按这个速度，窗口结束时还有富余
- **警告** —— 按这个速度，会在重置前用完
- **危急** —— 已经越过那个点了

打开弹窗能看到判断依据：实测的速度、投影的结果、以及距离窗口翻篇还有多久。

## 数字从哪来

| 来源                         | 提供什么                    | 怎么拿到的                                           |
| ---------------------------- | --------------------------- | ---------------------------------------------------- |
| Claude Code / Codex 会话日志 | token、模型、每次请求的成本 | 从 CLI 本地写的 JSONL 文件导入                       |
| Claude Desktop 套餐历史      | 五小时和每周的百分比        | 本地 JSON，由该 app 自己刷新                         |
| Claude Code 状态栏           | 百分比**以及**重置时刻      | 本地缓存，在会话渲染状态栏时写入                     |
| Codex OAuth                  | 订阅额度                    | 用你已有的 OAuth 会话签名请求                        |
| Coding Plan 接口             | 套餐额度与剩余重置次数      | Kimi、GLM、MiniMax 用 API key；火山方舟用 AK/SK 签名 |
| 供应商账单接口               | 每把 key 的花费与额度上限   | 用你保存的 key 直接调用                              |

没有本地代理，也不拦截任何请求。工具没写到磁盘上、接口也不报告的东西，这个 app 就是不知道。

## 分类视图

三个 Tab 共用同一个时间范围 —— 今天、7 天、30 天或一年：

- **Providers** —— 每个 Provider 的花费与 token，配 12 个月的每日活跃热力图和按小时/按天的趋势图
- **Models** —— 钱到底花在哪些模型上
- **Agents** —— 是哪个工具花的；可以显式绑定 Agent 与 Provider，处理那些否则无法归属的流量

每一次请求都可以展开查看，成本按你掌控的定价重算：可以刷新官方价目表，也可以为任一模型在某个 Provider 下单独覆盖价格。

## 安装

目前还没有发布任何 release，需要自己构建 —— 要求 macOS 12 及以上：

```bash
pnpm install && pnpm build:local:mac
```

这会构建并签名，产物在 `release/tauri-target/release/bundle/macos/LLM Usage Bar.app`，然后停下。去掉 `--build-only`（即直接跑 `./scripts/build_and_run.sh`）则会顺带装进 `/Applications` 并启动，还带一道校验 —— 新包起不来就自动恢复上一个版本。

> **升级是单向的。** app 首次启动会把数据库向前迁移，迁移前自动备份。一旦迁移完成，旧版本就再也打不开它了 —— 版本上限会直接拒绝，而不是冒险去写。装之前想清楚。

## 数据都在本地

| 路径                                | 内容                                         |
| ----------------------------------- | -------------------------------------------- |
| `~/.llm-usage-bar/llm-usage-bar.db` | SQLite —— 用量事件、Provider、定价、额度快照 |
| `~/.llm-usage-bar/settings.json`    | 设备级 UI 偏好                               |
| `~/.llm-usage-bar/backups/`         | 迁移前自动备份，默认保留最近 10 份           |
| `~/.llm-usage-bar/logs/`            | 应用日志                                     |

API key 存在 macOS 钥匙串里，**不进数据库，也不进日志**。花费数字从不写入日志文件。

可选的同步 —— 数据库可以放在自定义配置目录（iCloud、Dropbox、OneDrive、NAS），也可以推到 WebDAV 或 S3 兼容存储。默认关闭。

## 常见问题

<details>
<summary><strong>我需要改变使用 Claude Code 或 Codex 的方式吗？</strong></summary>

不需要。这个 app 读的是那些工具本来就在写的文件，不代理、不注入、不改写任何东西。就算你把它卸载了，你的 CLI 也不受影响。

</details>

<details>
<summary><strong>为什么某个 Provider 没有花费数字？</strong></summary>

因为花费来自供应商自己的账单接口，只有接好接口的预设才报得出来。目前接好的是 OpenRouter（`GET /api/v1/key`，作用域就是发起调用的那把 key）。其他供应商需要先把各自的接口接上。一个既没有接口、会话日志里也归不到它头上的 Provider，显示为空是正常的。

</details>

<details>
<summary><strong>为什么某把 key 的数字被标注成「属于已替换的凭据」？</strong></summary>

因为事实如此。替换一把 key 并不会把旧 key 已经产生的花费追溯性地转移过来，所以这些数字会被明确标出，而不是悄悄并进当前的合计里。

</details>

<details>
<summary><strong>Claude 显示了百分比，却没有重置时间，为什么？</strong></summary>

Claude 的两个本地数据源里只有一个带重置时刻 —— Claude Code 的状态栏桥接，它只在终端会话渲染状态栏时写入。Claude Desktop 的套餐历史有百分比，但从来不带重置时刻。app 见到一次就会把它锁存下来，一直用到它过期为止；但如果那个桥接从没跑过，就没有东西可锁存 —— 这时它会明说，而不是显示一个占位符。

</details>

<details>
<summary><strong>能跟踪在另一台机器上使用的订阅吗？</strong></summary>

不能。所有数据都来自本机的本地文件，以及作用域限定在单把 key 的接口。别处用的 key 对按 key 计的接口是不可见的，另一台机器的会话日志也不在这里，无从导入。

</details>

<details>
<summary><strong>界面支持哪些语言？</strong></summary>

English、简体中文、繁體中文、日本語。

</details>

## 文档

- [更新日志](CHANGELOG.md)
- [贡献指南](CONTRIBUTING.md) · [安全策略](SECURITY.md) · [支持](SUPPORT.md)

> `docs/user-manual/` 讲的仍然是已经移除的 Provider 切换、代理、MCP、Prompts 与 Skills 功能 —— 在重写之前不从这里链接。

## 技术栈

[Tauri 2](https://tauri.app/) · Rust · React 19 · TypeScript · SQLite

## 致谢

LLM Usage Bar 起初是 [CC Switch](https://github.com/farion1231/cc-switch)（作者 Jason Young）的 fork，至今仍构建在它的 Provider、存储、同步与计量基础之上。CC Switch 的 Provider 切换、代理、MCP、Prompts 与 Skills 功能已被移除；quota、ingestion、aggregation、dashboard、routing 与 native bridge 各层为本项目新增。

## 许可证

[MIT](LICENSE)
