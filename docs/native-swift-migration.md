# Swift 原生迁移路线

项目采用“绞杀者”方式逐步迁移：Tauri 版本在原生版本达到同等能力之前继续作为可发布应用，避免一次性重写造成数据丢失或功能倒退。

## 第一阶段（当前）

- `native/` 是独立 Swift Package，可构建一个 macOS 13+ 的 `MenuBarExtra` 应用。
- `UsageCore` 先定义与界面无关的用量快照、额度窗口和红绿灯状态，并提供带 schema 版本的原子 JSON 快照存储。
- 原生菜单栏已只读接入 Rust 状态栏桥生成的 `~/.llm-usage-bar/runtime/claude-statusline-quota.json`，会选择最新且未过期的 Claude 窗口，并支持手动刷新；目前不读取正式数据库，也不接触钥匙串。

构建与测试：

```bash
cd native
swift test
swift build
```

在 macOS 上运行开发版本：

```bash
cd native
swift run LLMUsageBarNative
```

## 后续阶段

1. **只读数据桥**：由现有 Rust 核心导出版本化快照，Swift 只读展示；用契约样例验证 TypeScript、Rust、Swift 三端字段一致。
2. **原生菜单栏等价**：迁移额度卡片、刷新状态、重置倒计时和可访问性；与 Tauri 菜单栏逐项视觉验收。
3. **原生主窗口**：按 Providers、Models、Agents 的顺序迁移查询与图表，保留 Rust 数据层和现有 SQLite schema。
4. **系统能力迁移**：最后迁移钥匙串、自动启动、更新、通知和文件选择。这些高风险能力在行为测试完备前不替换。
5. **移除 WebView**：仅当数据兼容、升级/降级策略和全部验收项通过后，才删除 React/Tauri 展示层；Rust 核心是否保留由当时的包体、性能和维护成本数据决定。

## 迁移约束

- 原生应用不得直接写现有 SQLite 数据库，直到 Rust 与 Swift 对 migration、备份和版本上限具有同一套契约测试。
- API key 继续只保存在 macOS 钥匙串；快照和日志不得包含凭据。
- 每个阶段都必须保留可回退到 Tauri 版本的发布路径，并使用相同的产品身份和数据目录。
