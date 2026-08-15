//! T20:依赖方向守卫 —— 让 T17 划好的模块边界不被慢慢磨平。
//!
//! 守的是 [模块化设计](../../docs/design/2026-08-14-modular-core-and-providers.md)
//! §9.3 第一条:
//!
//! ```text
//! model ← store ← {ingest, quota, route} ← api
//! ```
//!
//! T17 已经把目录搬成了这个形状,但**没有任何东西阻止三个月后有人往 `model/` 里
//! 写一行 `use crate::api::...`**。这个测试就是那个阻止者。
//!
//! ## 两条设计决定,改之前先读
//!
//! 1. **下面这张 `LAYER_RULES` 表是「谁是地基、谁是可拆的功能域」这份信息的唯一
//!    权威处。** 设计文档明确不用 `features/` 之类的目录前缀来承载它 —— 目录名是
//!    装饰,守卫是机器检查的。所以加一层就是往表里加一行。
//!
//! 2. **断言是双向的**(见 `extension_seam_dependencies_match_allowlist`):
//!    新增违规会失败,**修好了却不从 allowlist 删掉也会失败**。只做「不许新增」的
//!    单向守卫会烂掉 —— allowlist 会变成一张只增不减的垃圾场。
//!
//! ## 为什么大多数行是 `Inactive`
//!
//! 本次(T20)真正打开的只有 `model` 和 `store` 两条。其余各层的
//! `allowed_dependencies` 已经按设计写好,但 `enforcement` 是 `Inactive` ——
//! 因为现存违规量太大,一上来就开会得到一张巨型 allowlist,守卫就失去意义了。
//! **它们是给后续任务预留的开关,不是已经生效的保护。** 别误以为 `config`
//! 或 `usage` 现在被守着。
//!
//! ## `sync` / `providers` 两行
//!
//! 这两个模块由 T26 建立,现在还不存在。标成 `T26WhenPresent`:目录不存在时
//! 静默跳过,**一旦 T26 落地就自动开始扫描,这个文件不用改一行**。
//!
//! ## 已知违规见 `ALLOWLIST`
//!
//! 每一条都记了类型与成因。**红线:不要为了让 allowlist 空掉去改业务代码** ——
//! 那份清单本身就是下一个任务的输入(`model/domain.rs` 那条见 HANDOFF §15.1)。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const STORE: &[&str] = &["model", "store", "error", "http_client", "product_identity"];

const CONFIG: &[&str] = &[
    "model",
    "config",
    "error",
    "http_client",
    "product_identity",
];

const SECRETS: &[&str] = &[
    "model",
    "secrets",
    "error",
    "http_client",
    "product_identity",
];

const USAGE_DOMAIN: &[&str] = &[
    "model",
    "store",
    "config",
    "secrets",
    "error",
    "http_client",
    "product_identity",
    "usage",
    "ingest",
    "quota",
    "route",
];

const SYNC: &[&str] = &[
    "model",
    "store",
    "config",
    "secrets",
    "error",
    "http_client",
    "product_identity",
    "sync",
];

const API: &[&str] = &[
    "model",
    "store",
    "config",
    "secrets",
    "error",
    "http_client",
    "product_identity",
    "usage",
    "ingest",
    "quota",
    "route",
    "sync",
    "providers",
    "api",
];

const SYNC_BLOCKED: &[&str] = &["usage", "ingest", "quota", "route", "providers", "api"];

const PROVIDERS: &[&str] = &[
    "model",
    "store",
    "config",
    "secrets",
    "error",
    "http_client",
    "product_identity",
    "quota",
    "providers",
];

#[derive(Clone, Copy)]
enum Enforcement {
    /// 规则已写好但不检查 —— 现存违规太多,开了就是一张巨型 allowlist。
    /// 留给后续任务打开,见文件头。
    Inactive,
    /// 白名单制:`allowed_dependencies` 之外的一切都是违规。用于地基层。
    AllowOnly,
    /// 黑名单制:只有列出的那些是违规。用于「大体正确、只堵几个方向」的层。
    Deny(&'static [&'static str]),
}

#[derive(Clone, Copy)]
enum Availability {
    /// 模块必须存在;不存在说明有人搬走了目录而没同步这张表,直接 panic。
    Required,
    /// T26 之后才存在。不存在时静默跳过,存在时自动开始扫描。
    T26WhenPresent,
}

struct LayerRule {
    module: &'static str,
    allowed_dependencies: &'static [&'static str],
    enforcement: Enforcement,
    availability: Availability,
}

const LAYER_RULES: &[LayerRule] = &[
    LayerRule {
        module: "model",
        allowed_dependencies: &[],
        enforcement: Enforcement::AllowOnly,
        availability: Availability::Required,
    },
    LayerRule {
        module: "store",
        allowed_dependencies: STORE,
        enforcement: Enforcement::Deny(&["ingest", "quota", "route", "api", "usage"]),
        availability: Availability::Required,
    },
    LayerRule {
        module: "config",
        allowed_dependencies: CONFIG,
        enforcement: Enforcement::Inactive,
        availability: Availability::Required,
    },
    LayerRule {
        module: "secrets",
        allowed_dependencies: SECRETS,
        enforcement: Enforcement::Inactive,
        availability: Availability::Required,
    },
    LayerRule {
        module: "usage",
        allowed_dependencies: USAGE_DOMAIN,
        enforcement: Enforcement::Inactive,
        availability: Availability::Required,
    },
    LayerRule {
        module: "ingest",
        allowed_dependencies: USAGE_DOMAIN,
        enforcement: Enforcement::Inactive,
        availability: Availability::Required,
    },
    LayerRule {
        module: "quota",
        allowed_dependencies: USAGE_DOMAIN,
        enforcement: Enforcement::Inactive,
        availability: Availability::Required,
    },
    LayerRule {
        module: "route",
        allowed_dependencies: USAGE_DOMAIN,
        enforcement: Enforcement::Inactive,
        availability: Availability::Required,
    },
    LayerRule {
        module: "sync",
        allowed_dependencies: SYNC,
        enforcement: Enforcement::Deny(SYNC_BLOCKED),
        availability: Availability::T26WhenPresent,
    },
    LayerRule {
        module: "providers",
        allowed_dependencies: PROVIDERS,
        enforcement: Enforcement::AllowOnly,
        availability: Availability::T26WhenPresent,
    },
    LayerRule {
        module: "api",
        allowed_dependencies: API,
        enforcement: Enforcement::Inactive,
        availability: Availability::Required,
    },
];

struct ExpectedViolation {
    boundary: &'static str,
    file: &'static str,
    dependency: &'static str,
    kind: &'static str,
    reason: &'static str,
}

const ALLOWLIST: &[ExpectedViolation] = &[
    ExpectedViolation {
        boundary: "model",
        file: "src-tauri/src/model/domain.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "三个纯状态枚举仍位于 usage::status；拆分文件不在 T17 只搬不改范围内",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/backup.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "备份恢复流程直接调用 usage 迁移与校验逻辑",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/dao/agent_provider_bindings.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "DAO 复用 usage::system_providers 的系统 provider 判定",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/dao/binding_credentials.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "凭据 DAO 复用 usage::system_providers 的固定 API 判定",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/dao/provider_credentials.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "凭据 DAO 复用 usage::system_providers 的固定 API 判定",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/dao/provider_model_pricing.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "定价 DAO 复用 usage::usage_stats 的模型 ID 清理函数",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/dao/quota.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "quota DAO 复用 usage::usage_light_prediction 的快照滚动逻辑",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/dao/usage_providers.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "用量 provider DAO 复用 usage 的预算、系统 provider 与迁移逻辑",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/dao/usage_rollup.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "用量汇总 DAO 复用 usage 的过滤与预测清理逻辑",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/mod.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "数据库初始化依赖 usage::source_roots 解析运行时路径",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/schema.rs",
        dependency: "usage",
        kind: "地基上行依赖",
        reason: "schema 迁移调度直接调用 usage 的迁移与校验函数",
    },
    ExpectedViolation {
        boundary: "store",
        file: "src-tauri/src/store/tests.rs",
        dependency: "usage",
        kind: "测试边界上行依赖",
        reason: "store 集成测试直接驱动 usage 迁移并构造系统 provider fixture",
    },
];

type ViolationKey = (String, String, String);

fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn repository_relative(path: &Path) -> String {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri must have a repository parent");
    path.strip_prefix(repository_root)
        .expect("scanned source must be inside the repository")
        .to_string_lossy()
        .replace('\\', "/")
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        {
            let entry = entry.unwrap_or_else(|error| {
                panic!("cannot read an entry in {}: {error}", directory.display())
            });
            let path = entry.path();
            let file_type = entry
                .file_type()
                .unwrap_or_else(|error| panic!("cannot stat {}: {error}", path.display()));
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }

    files.sort();
    files
}

fn identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn grouped_crate_dependencies(
    source: &str,
    open_brace: usize,
    dependencies: &mut BTreeSet<String>,
) -> usize {
    let bytes = source.as_bytes();
    let mut index = open_brace + 1;
    let mut depth = 1;
    let mut expects_root = true;

    while index < bytes.len() && depth > 0 {
        match bytes[index] {
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth -= 1;
                index += 1;
            }
            b',' if depth == 1 => {
                expects_root = true;
                index += 1;
            }
            byte if depth == 1 && expects_root && byte.is_ascii_whitespace() => {
                index += 1;
            }
            byte if depth == 1 && expects_root && identifier_start(byte) => {
                let start = index;
                while index < bytes.len() && identifier_continue(bytes[index]) {
                    index += 1;
                }
                dependencies.insert(source[start..index].to_string());
                expects_root = false;
            }
            _ => {
                index += 1;
            }
        }
    }

    index
}

fn crate_dependencies(source: &str) -> BTreeSet<String> {
    let bytes = source.as_bytes();
    let mut dependencies = BTreeSet::new();
    let mut search_from = 0;

    while let Some(relative_start) = source[search_from..].find("crate::") {
        let start = search_from + relative_start;
        let has_identifier_prefix = start > 0 && identifier_continue(bytes[start - 1]);
        if has_identifier_prefix {
            search_from = start + "crate::".len();
            continue;
        }

        let dependency_start = start + "crate::".len();
        if dependency_start >= bytes.len() {
            break;
        }

        if bytes[dependency_start] == b'{' {
            search_from = grouped_crate_dependencies(source, dependency_start, &mut dependencies);
            continue;
        }

        if !identifier_start(bytes[dependency_start]) {
            search_from = dependency_start + 1;
            continue;
        }
        let end = bytes[dependency_start..]
            .iter()
            .position(|byte| !identifier_continue(*byte))
            .map_or(bytes.len(), |offset| dependency_start + offset);
        dependencies.insert(source[dependency_start..end].to_string());
        search_from = end;
    }

    dependencies
}

fn is_violation(rule: &LayerRule, dependency: &str) -> bool {
    match rule.enforcement {
        Enforcement::Inactive => false,
        Enforcement::AllowOnly => !rule.allowed_dependencies.contains(&dependency),
        Enforcement::Deny(blocked) => blocked.contains(&dependency),
    }
}

fn actual_violations() -> BTreeSet<ViolationKey> {
    let source_root = source_root();
    let mut violations = BTreeSet::new();

    for rule in LAYER_RULES {
        let module_root = source_root.join(rule.module);
        if !module_root.exists() {
            match rule.availability {
                Availability::T26WhenPresent => continue,
                Availability::Required => panic!(
                    "required module directory is missing: {}",
                    module_root.display()
                ),
            }
        }

        for file in rust_files(&module_root) {
            let source = fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
            for dependency in crate_dependencies(&source) {
                if is_violation(rule, &dependency) {
                    violations.insert((
                        rule.module.to_string(),
                        repository_relative(&file),
                        dependency,
                    ));
                }
            }
        }
    }

    violations
}

fn allowlist_violations() -> BTreeSet<ViolationKey> {
    let violations = ALLOWLIST
        .iter()
        .map(|entry| {
            assert!(
                !entry.kind.trim().is_empty(),
                "allowlist type must be recorded"
            );
            assert!(
                !entry.reason.trim().is_empty(),
                "allowlist reason must be recorded"
            );
            (
                entry.boundary.to_string(),
                entry.file.to_string(),
                entry.dependency.to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        violations.len(),
        ALLOWLIST.len(),
        "allowlist must not contain duplicate dependency edges"
    );
    violations
}

#[test]
fn scanner_finds_direct_inline_and_grouped_crate_paths() {
    let source = r#"
use crate::{
    api::{commands, router},
    usage::status,
};

fn probe() {
    let _ = crate::route::server::start;
}
"#;
    let expected = ["api", "route", "usage"]
        .into_iter()
        .map(str::to_string)
        .collect();

    assert_eq!(crate_dependencies(source), expected);
}

#[test]
fn extension_seam_dependencies_match_allowlist() {
    let actual = actual_violations();
    let allowlist = allowlist_violations();

    assert_eq!(
        actual, allowlist,
        "依赖方向违规集合变化；请先逐条审阅，再同步 allowlist。实际集合: {actual:#?}；allowlist: {allowlist:#?}"
    );
}
