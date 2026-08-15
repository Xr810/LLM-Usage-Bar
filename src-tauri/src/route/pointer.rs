//! Codex `model_provider` 指针的写入、启动检查与逃生。
//!
//! 为什么整个模块这么谨慎:用户当初正是被「工具反复复写 `~/.codex/config.toml`
//! 把配置写坏」坑过,本任务的全部意义就是只写一次、且写不坏。写入路径因此是:
//! 先备份 → 同目录临时文件 + rename 原子替换(绝不截断原文件)→ 读回校验,
//! 校验失败用备份还原并返回 Err。除顶层 `model_provider` 一个键和
//! `[model_providers.llm_usage_bar_router]` 一个段之外,文件其余字节一律不动。

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{value, DocumentMut, Item, Table};

use crate::error::AppError;

/// 写入 Codex 配置的 provider id,带 app 前缀,避免与用户自建 provider 撞名。
const ROUTER_PROVIDER_ID: &str = "llm_usage_bar_router";
/// 备份文件名后缀;逃生脚本 scripts/codex-unroute.sh 按同一 glob 找备份。
const BACKUP_NAME_SUFFIX: &str = "-before-router";

/// 把 Codex 的 model_provider 指向本地 router。
///
/// 只改**顶层的 model_provider 这一个键**,以及新增一个
/// [model_providers.llm_usage_bar_router] 段。文件里其余内容——MCP 服务器、
/// 已信任的项目、用户自己的注释——**一个字节都不能动**。
pub fn point_codex_at_router(port: u16) -> Result<(), AppError> {
    let config_path = crate::agent_paths::get_codex_config_dir().join("config.toml");
    point_codex_at_router_inner(&config_path, port)
}

/// 启动时检查指针是不是自己写的,由上层决定怎么提示用户。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerState {
    /// 指向我们的 router,一切正常。
    OursAndCurrent,
    /// 指向别处——用户手动改过(逃生),或从未配置过。
    NotOurs { current: Option<String> },
    /// 文件读不到或解析不了。
    Unreadable,
}

/// 启动时检查 Codex 的 model_provider 是不是指向我们的 router。
///
/// **只读不写**:发现不是自己写的绝不静默覆盖(设计文档决定 36),而是把状态
/// 返回给上层,由上层提示用户并标记「这段时间的用量没有经过 router」。
pub fn inspect_pointer() -> PointerState {
    let config_path = crate::agent_paths::get_codex_config_dir().join("config.toml");
    inspect_pointer_inner(&config_path)
}

/// `point_codex_at_router` 的路径可注入版本:测试用 tempfile 造临时文件,
/// 绝不碰用户真实的 `~/.codex/config.toml`。
fn point_codex_at_router_inner(config_path: &Path, port: u16) -> Result<(), AppError> {
    // 读原文件。config.toml 不存在时按首次写入处理:没有可备份的旧内容,
    // 校验失败时把新建的文件删掉,还原成「不存在」。
    let (original, existed) = match fs::read_to_string(config_path) {
        Ok(text) => (text, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), false),
        Err(e) => return Err(AppError::io(config_path, e)),
    };

    let mut doc = if existed {
        original.parse::<DocumentMut>().map_err(|e| {
            AppError::Config(format!(
                "Codex config.toml 解析失败({}): {e}",
                config_path.display()
            ))
        })?
    } else {
        DocumentMut::new()
    };

    // 改动 1/2:替换顶层 model_provider 这一个键。
    doc["model_provider"] = value(ROUTER_PROVIDER_ID);

    // 改动 2/2:追加 [model_providers.llm_usage_bar_router] 段。
    // 不能走 doc["model_providers"][id] = ... 的索引链:toml_edit 的索引链在父键
    // 缺失时会先垫一个 Item::None,而 Item::None 被再次索引时会被转成 inline
    // table,写出来变成 model_providers = { ... },结构就错了(实测:读回校验
    // 会失败)。配置文件是不可信输入,model_providers 不是表时也要显式拒绝,
    // 而不是在索引时 panic。
    let root = doc.as_table_mut();
    if let Some(item) = root.get("model_providers") {
        if !item.is_table() {
            return Err(AppError::Config(format!(
                "{}: 顶层 model_providers 不是表,拒绝写入以免破坏配置",
                config_path.display()
            )));
        }
    } else {
        let mut providers = Table::new();
        // 隐式空表序列化时不打印表头,与「只有 [model_providers.<id>] 段」的
        // 既有配置的渲染方式一致。
        providers.set_implicit(true);
        root.insert("model_providers", Item::Table(providers));
    }
    let providers = root
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
        .expect("model_providers 在上面刚被校验或创建为表");
    let mut section = Table::new();
    section["name"] = value("LLM Usage Bar Router");
    // 末尾 /v1 是硬契约(README §2.2):Codex 会往 base_url 后接 /responses。
    section["base_url"] = value(format!("http://127.0.0.1:{port}/v1"));
    section["wire_api"] = value("responses");
    providers.insert(ROUTER_PROVIDER_ID, Item::Table(section));

    let new_text = doc.to_string();

    // 1) 写前备份(只对已存在的文件)。
    let backup_path = if existed {
        let backup = backup_path_for(config_path)?;
        fs::copy(config_path, &backup).map_err(|e| AppError::io(&backup, e))?;
        Some(backup)
    } else {
        None
    };

    // 2) 原子写入:同目录临时文件 + rename,不直接截断原文件。
    let tmp_path = temp_sibling_path(config_path);
    fs::write(&tmp_path, new_text.as_bytes()).map_err(|e| AppError::io(&tmp_path, e))?;
    if existed {
        // 继承原文件权限:config.toml 里可能带着 token,不能从 0600 变 0644。
        if let Ok(metadata) = fs::metadata(config_path) {
            let _ = fs::set_permissions(&tmp_path, metadata.permissions());
        }
    }
    if let Err(e) = fs::rename(&tmp_path, config_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(AppError::io(config_path, e));
    }

    #[cfg(test)]
    {
        // 测试 4 的故障注入点:模拟 rename 之后、读回校验之前磁盘内容被破坏。
        // 生产构建里整个块被 cfg 掉,不影响任何运行路径。
        if test_support::take_corrupt_after_rename() {
            let _ = fs::write(config_path, b"this is not = valid [toml");
        }
    }

    // 3) 读回校验:能被 TOML 解析、且 model_provider 与 base_url 确实是新值。
    if let Err(e) = verify_pointer_file(config_path, port) {
        // 校验失败:用备份还原,返回 Err。
        if let Some(backup) = &backup_path {
            return match restore_from_backup(config_path, backup) {
                Ok(()) => Err(e),
                Err(restore) => Err(AppError::Config(format!(
                    "{e};且用备份还原也失败: {restore}"
                ))),
            };
        }
        // 首次写入场景没有备份,把写坏的文件删掉,恢复成「不存在」。
        let _ = fs::remove_file(config_path);
        return Err(e);
    }

    Ok(())
}

/// 读回校验:文件存在、能被 TOML 解析、model_provider 与 base_url 都是刚写的值。
fn verify_pointer_file(config_path: &Path, port: u16) -> Result<(), AppError> {
    let text = fs::read_to_string(config_path).map_err(|e| AppError::io(config_path, e))?;
    let doc: DocumentMut = text.parse().map_err(|e| {
        AppError::Config(format!(
            "写入后的 config.toml 读回解析失败({}): {e}",
            config_path.display()
        ))
    })?;

    if doc.get("model_provider").and_then(Item::as_str) != Some(ROUTER_PROVIDER_ID) {
        return Err(AppError::Config(format!(
            "写入后的 model_provider 不是 {ROUTER_PROVIDER_ID}: {}",
            config_path.display()
        )));
    }

    let expected_base_url = format!("http://127.0.0.1:{port}/v1");
    let base_url = doc
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|table| table.get(ROUTER_PROVIDER_ID))
        .and_then(Item::as_table)
        .and_then(|table| table.get("base_url"))
        .and_then(Item::as_str);
    if base_url != Some(expected_base_url.as_str()) {
        return Err(AppError::Config(format!(
            "写入后的 [model_providers.{ROUTER_PROVIDER_ID}] base_url 不符: {}",
            config_path.display()
        )));
    }

    Ok(())
}

/// 把备份原子地还原回配置路径(同样走临时文件 + rename)。
fn restore_from_backup(config_path: &Path, backup_path: &Path) -> Result<(), AppError> {
    let tmp_path = temp_sibling_path(config_path);
    fs::copy(backup_path, &tmp_path).map_err(|e| AppError::io(&tmp_path, e))?;
    if let Err(e) = fs::rename(&tmp_path, config_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(AppError::io(config_path, e));
    }
    Ok(())
}

/// 生成备份路径:config.toml.bak-<yyyymmdd-HHMMSS>-before-router,
/// 同秒冲突时加序号,避免覆盖更早的备份。
fn backup_path_for(config_path: &Path) -> Result<PathBuf, AppError> {
    let file_name = config_path
        .file_name()
        .ok_or_else(|| AppError::Config(format!("配置路径没有文件名: {}", config_path.display())))?
        .to_string_lossy();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    for i in 0.. {
        let candidate = if i == 0 {
            dir.join(format!("{file_name}.bak-{stamp}{BACKUP_NAME_SUFFIX}"))
        } else {
            dir.join(format!("{file_name}.bak-{stamp}-{i}{BACKUP_NAME_SUFFIX}"))
        };
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    unreachable!("for 0.. 永远在找到空闲名字时 return")
}

/// 与目标同目录的临时文件路径(保证 rename 不跨文件系统、原子生效)。
fn temp_sibling_path(config_path: &Path) -> PathBuf {
    let file_name = config_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "config.toml".into());
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%f");
    let dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!("{file_name}.tmp-{}-{stamp}", std::process::id()))
}

/// `inspect_pointer` 的路径可注入版本,只读不写。
fn inspect_pointer_inner(config_path: &Path) -> PointerState {
    let text = match fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(_) => return PointerState::Unreadable,
    };
    let doc: DocumentMut = match text.parse() {
        Ok(doc) => doc,
        Err(_) => return PointerState::Unreadable,
    };
    match doc.get("model_provider").and_then(Item::as_str) {
        Some(id) if id == ROUTER_PROVIDER_ID => PointerState::OursAndCurrent,
        Some(id) => PointerState::NotOurs {
            current: Some(id.to_string()),
        },
        None => PointerState::NotOurs { current: None },
    }
}

#[cfg(test)]
mod test_support {
    use std::cell::Cell;

    thread_local! {
        /// 测试 4 专用:置 true 后,下一次写入在 rename 之后把目标文件写成坏 TOML。
        pub(super) static CORRUPT_AFTER_RENAME: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn take_corrupt_after_rename() -> bool {
        CORRUPT_AFTER_RENAME.with(|flag| flag.replace(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const PORT: u16 = 43123;

    /// 含注释、MCP 段、多个 provider 的完整配置(测试 1/2/4/5 共用)。
    const FIXTURE_FULL: &str = r#"# 顶层注释:这是用户自己写的,必须原样保留

model_provider = "openai"

[tools]
web_search = true

[mcp_servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

# 第二个 provider
[model_providers.packyapi]
name = "packyapi"
base_url = "https://www.packyapi.ai/v1"
wire_api = "responses"

[model_providers.ollama]
name = "ollama"
base_url = "http://localhost:11434/v1"
"#;

    /// FIXTURE_FULL 写入后的逐字节期望:只有 model_provider 一行和末尾新增的段
    /// 与原文不同,其余(注释、MCP 段、provider 键序、空行)全部原样。
    /// 这个字面量是硬编码的,不是用 toml_edit 算出来的——算出来的期望会与
    /// 实现同错,等于没测。
    const EXPECTED_FULL: &str = r#"# 顶层注释:这是用户自己写的,必须原样保留

model_provider = "llm_usage_bar_router"

[tools]
web_search = true

[mcp_servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

# 第二个 provider
[model_providers.packyapi]
name = "packyapi"
base_url = "https://www.packyapi.ai/v1"
wire_api = "responses"

[model_providers.ollama]
name = "ollama"
base_url = "http://localhost:11434/v1"

[model_providers.llm_usage_bar_router]
name = "LLM Usage Bar Router"
base_url = "http://127.0.0.1:43123/v1"
wire_api = "responses"
"#;

    /// 测试里绝不碰真实 ~/.codex/config.toml:全部用 tempfile 造临时文件。
    fn config_in(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("config.toml");
        fs::write(&path, content).unwrap();
        path
    }

    fn backup_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("config.toml.bak-") && name.ends_with("-before-router"))
            .collect();
        names.sort();
        names
    }

    fn tmp_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("config.toml.tmp-"))
            .collect();
        names.sort();
        names
    }

    /// 测试 1:写入后除 model_provider 那行和新增段外逐字节相同。
    #[test]
    fn write_preserves_every_other_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path(), FIXTURE_FULL);

        point_codex_at_router_inner(&path, PORT).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(written, EXPECTED_FULL, "除指针一行与新增段外必须逐字节相同");
        // 原文与结果必须真的不同,防止上面两个常量复制粘贴成一样的假绿。
        assert_ne!(written, FIXTURE_FULL);
    }

    /// 测试 2:已有 model_provider → 就地替换,不追加第二个。
    #[test]
    fn write_replaces_existing_model_provider_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path(), "model_provider = \"openai\"\n");

        point_codex_at_router_inner(&path, PORT).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            "model_provider = \"llm_usage_bar_router\"\n\n[model_providers.llm_usage_bar_router]\nname = \"LLM Usage Bar Router\"\nbase_url = \"http://127.0.0.1:43123/v1\"\nwire_api = \"responses\"\n"
        );
        // 顶层 model_provider 赋值行恰好一条,没有追加第二个。
        let assignments = written
            .lines()
            .filter(|line| *line == "model_provider = \"llm_usage_bar_router\"")
            .count();
        assert_eq!(assignments, 1, "不允许出现第二个 model_provider 键");
    }

    /// 测试 3:没有 model_provider → 插在第一个 [table] 之前。
    #[test]
    fn write_without_model_provider_inserts_before_first_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(
            dir.path(),
            "[model_providers.packyapi]\nname = \"packyapi\"\nbase_url = \"https://www.packyapi.ai/v1\"\nwire_api = \"responses\"\n",
        );

        point_codex_at_router_inner(&path, PORT).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            "model_provider = \"llm_usage_bar_router\"\n[model_providers.packyapi]\nname = \"packyapi\"\nbase_url = \"https://www.packyapi.ai/v1\"\nwire_api = \"responses\"\n\n[model_providers.llm_usage_bar_router]\nname = \"LLM Usage Bar Router\"\nbase_url = \"http://127.0.0.1:43123/v1\"\nwire_api = \"responses\"\n"
        );
        // model_provider 必须出现在第一个 [ 之前。
        let model_provider_line = written
            .lines()
            .position(|line| line.starts_with("model_provider"))
            .unwrap();
        let first_table_line = written
            .lines()
            .position(|line| line.starts_with('['))
            .unwrap();
        assert!(model_provider_line < first_table_line);
    }

    /// 测试 4:读回校验失败 → 用备份还原,返回 Err。
    #[test]
    fn verify_failure_restores_backup_and_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path(), FIXTURE_FULL);

        // 注入:rename 之后把目标文件写成坏 TOML,逼读回校验失败。
        test_support::CORRUPT_AFTER_RENAME.with(|flag| flag.set(true));
        let result = point_codex_at_router_inner(&path, PORT);

        assert!(result.is_err(), "读回校验失败必须返回 Err");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            FIXTURE_FULL,
            "校验失败后必须用备份把文件逐字节还原"
        );
        let backups = backup_files(dir.path());
        assert_eq!(backups.len(), 1, "写前必须留一份备份");
        assert_eq!(
            fs::read_to_string(dir.path().join(&backups[0])).unwrap(),
            FIXTURE_FULL
        );
        assert!(tmp_files(dir.path()).is_empty(), "不能留下临时文件");
    }

    /// 测试 5:inspect_pointer 对我们写的 → OursAndCurrent。
    #[test]
    fn inspect_pointer_recognizes_our_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path(), FIXTURE_FULL);

        point_codex_at_router_inner(&path, PORT).unwrap();
        assert_eq!(inspect_pointer_inner(&path), PointerState::OursAndCurrent);
    }

    /// 测试 6:inspect_pointer 对用户手改成 packyapi 的 → NotOurs。
    #[test]
    fn inspect_pointer_reports_user_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path(), "model_provider = \"packyapi\"\n");

        assert_eq!(
            inspect_pointer_inner(&path),
            PointerState::NotOurs {
                current: Some("packyapi".to_string())
            }
        );

        // 从未配置过 model_provider 也是 NotOurs,current 为 None。
        let other_dir = tempfile::tempdir().unwrap();
        let missing = config_in(
            other_dir.path(),
            "[model_providers.packyapi]\nname = \"packyapi\"\n",
        );
        assert_eq!(
            inspect_pointer_inner(&missing),
            PointerState::NotOurs { current: None }
        );
    }

    /// 测试 7:inspect_pointer 对损坏的 TOML → Unreadable,不 panic。
    #[test]
    fn inspect_pointer_reports_unreadable_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path(), "this is not = valid [toml");

        assert_eq!(inspect_pointer_inner(&path), PointerState::Unreadable);

        // 文件不存在同样返回 Unreadable(启动时可能还没跑过 codex)。
        assert_eq!(
            inspect_pointer_inner(&dir.path().join("does-not-exist.toml")),
            PointerState::Unreadable
        );
    }

    /// 附加:顶层 model_providers 不是表时拒绝写入、文件不动、也不留备份。
    /// 不拒绝的话索引链会在运行时 panic——用户配置是不可信输入。
    #[test]
    fn write_refuses_non_table_model_providers() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = "model_providers = \"not-a-table\"\nmodel_provider = \"openai\"\n";
        let path = config_in(dir.path(), fixture);

        let result = point_codex_at_router_inner(&path, PORT);

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            fixture,
            "拒绝时不许动文件"
        );
        assert!(
            backup_files(dir.path()).is_empty(),
            "还没备份就拒绝,不该留备份"
        );
        assert!(tmp_files(dir.path()).is_empty());
    }
}
