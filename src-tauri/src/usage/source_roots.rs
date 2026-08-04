use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UsageSourceOverrides {
    pub claude_config_dir: Option<PathBuf>,
    pub codex_config_dir: Option<PathBuf>,
    pub gemini_config_dir: Option<PathBuf>,
    pub opencode_db_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsageSourceRoots {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub gemini: PathBuf,
    pub opencode: PathBuf,
}

impl UsageSourceRoots {
    pub(crate) fn resolve_for_migration(home: &Path, overrides: UsageSourceOverrides) -> Self {
        let claude = resolve_against_home(
            home,
            overrides
                .claude_config_dir
                .unwrap_or_else(|| home.join(".claude")),
        )
        .join("projects");
        let codex = resolve_against_home(
            home,
            overrides
                .codex_config_dir
                .unwrap_or_else(|| home.join(".codex")),
        );
        let gemini = resolve_against_home(
            home,
            overrides
                .gemini_config_dir
                .unwrap_or_else(|| home.join(".gemini")),
        )
        .join("tmp");
        let opencode_db = resolve_against_home(
            home,
            overrides
                .opencode_db_path
                .unwrap_or_else(|| home.join(".local/share/opencode/opencode.db")),
        );
        let opencode = opencode_db
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".local/share/opencode"));

        Self {
            claude,
            codex,
            gemini,
            opencode,
        }
    }

    pub(crate) fn resolve_runtime() -> Self {
        let home = crate::config::get_home_dir();
        Self::resolve_for_migration(
            &home,
            UsageSourceOverrides {
                claude_config_dir: crate::settings::get_claude_override_dir(),
                codex_config_dir: crate::settings::get_codex_override_dir(),
                gemini_config_dir: crate::settings::get_gemini_override_dir(),
                opencode_db_path: Some(crate::agent_paths::get_opencode_db_path()),
            },
        )
    }
}

fn resolve_against_home(home: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        home.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn defaults_are_scoped_below_each_external_product_root() {
        let home = PathBuf::from("/Users/tester");
        let roots = UsageSourceRoots::resolve_for_migration(&home, UsageSourceOverrides::default());

        assert_eq!(roots.claude, home.join(".claude/projects"));
        assert_eq!(roots.codex, home.join(".codex"));
        assert_eq!(roots.gemini, home.join(".gemini/tmp"));
        assert_eq!(roots.opencode, home.join(".local/share/opencode"));
    }

    #[test]
    fn explicit_dirs_replace_only_their_own_source_roots() {
        let home = PathBuf::from("/Users/tester");
        let overrides = UsageSourceOverrides {
            claude_config_dir: Some(PathBuf::from("/Volumes/logs/claude")),
            codex_config_dir: Some(PathBuf::from("/Volumes/logs/codex")),
            gemini_config_dir: Some(PathBuf::from("/Volumes/logs/gemini")),
            opencode_db_path: Some(PathBuf::from("/Volumes/logs/opencode/state.db")),
        };
        let roots = UsageSourceRoots::resolve_for_migration(&home, overrides);

        assert_eq!(roots.claude, PathBuf::from("/Volumes/logs/claude/projects"));
        assert_eq!(roots.codex, PathBuf::from("/Volumes/logs/codex"));
        assert_eq!(roots.gemini, PathBuf::from("/Volumes/logs/gemini/tmp"));
        assert_eq!(roots.opencode, PathBuf::from("/Volumes/logs/opencode"));
    }

    #[test]
    fn relative_overrides_resolve_against_home_without_touching_the_filesystem() {
        let home = PathBuf::from("/Users/tester");
        let overrides = UsageSourceOverrides {
            claude_config_dir: Some(PathBuf::from("custom/claude")),
            opencode_db_path: Some(PathBuf::from("custom/opencode.db")),
            ..UsageSourceOverrides::default()
        };
        let roots = UsageSourceRoots::resolve_for_migration(&home, overrides);

        assert_eq!(roots.claude, home.join("custom/claude/projects"));
        assert_eq!(roots.opencode, home.join("custom"));
    }
}
