//! CLI wire types. Keep response fields snake_case and WSL input camelCase.
#[derive(serde::Serialize)]
pub struct ToolVersion {
    pub(super) name: String,
    pub(super) version: Option<String>,
    pub(super) latest_version: Option<String>,
    pub(super) error: Option<String>,
    /// Located but --version failed; distinct from a missing installation.
    pub(super) installed_but_broken: bool,
    pub(super) env_type: String,
    pub(super) wsl_distro: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WslShellPreferenceInput {
    #[serde(default)]
    pub wsl_shell: Option<String>,
    #[serde(default)]
    pub wsl_shell_flag: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct ToolInstallation {
    /// Entry as seen in PATH, before resolving symlinks.
    pub(super) path: String,
    pub(super) version: Option<String>,
    pub(super) runnable: bool,
    pub(super) error: Option<String>,
    pub(super) source: String,
    pub(super) is_path_default: bool,
    /// Reuse the canonical path from enumeration for anchored updates. Never
    /// resolve it again: the symlink could change between those two steps.
    /// This internal path is not part of the frontend contract.
    #[serde(skip)]
    pub(super) real: std::path::PathBuf,
}

#[derive(Debug, serde::Serialize)]
pub struct ToolInstallationReport {
    pub(super) tool: String,
    pub(super) installs: Vec<ToolInstallation>,
    /// Two or more installs with different versions or runnable states.
    pub(super) is_conflict: bool,
    /// Any two or more installs, even when they agree.
    pub(super) needs_confirmation: bool,
    /// Display only; execution always regenerates the command on the backend.
    pub(super) command: String,
    pub(super) anchored: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_wire_contract_matches_shared_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/cli-contract.json"))
                .unwrap();
        let version = ToolVersion {
            name: "codex".into(),
            version: None,
            latest_version: Some("1.2.3".into()),
            error: Some("node unavailable".into()),
            installed_but_broken: true,
            env_type: "wsl".into(),
            wsl_distro: Some("Ubuntu".into()),
        };
        assert_eq!(serde_json::to_value(version).unwrap(), fixture["version"]);
        let report = ToolInstallationReport {
            tool: "codex".into(),
            installs: vec![ToolInstallation {
                path: "/opt/bin/codex".into(),
                version: Some("1.2.1".into()),
                runnable: true,
                error: None,
                source: "npm".into(),
                is_path_default: true,
                real: "/private/internal/path".into(),
            }],
            is_conflict: false,
            needs_confirmation: false,
            command: "codex update".into(),
            anchored: true,
        };
        assert_eq!(serde_json::to_value(report).unwrap(), fixture["report"]);
        let input: WslShellPreferenceInput =
            serde_json::from_value(fixture["wsl"].clone()).unwrap();
        assert_eq!(input.wsl_shell.as_deref(), Some("zsh"));
        assert_eq!(input.wsl_shell_flag.as_deref(), Some("-ic"));
        for value in [
            serde_json::json!({}),
            serde_json::json!({"wslShell":null,"wslShellFlag":null}),
        ] {
            let input: WslShellPreferenceInput = serde_json::from_value(value).unwrap();
            assert!(input.wsl_shell.is_none());
            assert!(input.wsl_shell_flag.is_none());
        }
    }
}
