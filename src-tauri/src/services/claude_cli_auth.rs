use crate::error::AppError;
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CLAUDE_BINARY: &str = "claude";
const CLAUDE_STATUS_ARGS: [&str; 2] = ["auth", "status"];
const CLAUDE_LOGOUT_ARGS: [&str; 2] = ["auth", "logout"];
const CLAUDE_LOGIN_COMMAND_LINE: &str = "claude auth login";
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeSubscriptionType {
    Pro,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeAuthMethod {
    ApiKey,
    ClaudeAccount,
    Other,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCliAuthStatus {
    pub installed: bool,
    pub authenticated: bool,
    pub auth_method: Option<ClaudeAuthMethod>,
    pub subscription_type: Option<ClaudeSubscriptionType>,
    pub quota_availability: &'static str,
    pub error_code: Option<String>,
}

pub struct ClaudeAuthCommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
}

pub type ClaudeAuthCommandFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ClaudeAuthCommandOutput, AppError>> + Send + 'a>>;

pub trait ClaudeAuthCommandRunner: Send + Sync {
    fn status(&self) -> ClaudeAuthCommandFuture<'_>;
    fn launch_login(&self) -> Result<(), AppError>;
    fn logout(&self) -> ClaudeAuthCommandFuture<'_>;
}

#[derive(Default)]
pub struct ProductionClaudeAuthCommandRunner;

fn command_error(code: &'static str) -> AppError {
    AppError::Message(code.to_string())
}

fn claude_binary_candidates_for_home(home: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(CLAUDE_BINARY)];

    #[cfg(not(target_os = "windows"))]
    {
        for relative in [
            ".local/bin/claude",
            ".volta/bin/claude",
            ".asdf/shims/claude",
            ".local/share/mise/shims/claude",
            ".config/mise/shims/claude",
            ".npm-global/bin/claude",
            ".npm-packages/bin/claude",
            ".local/share/pnpm/claude",
            "Library/pnpm/claude",
        ] {
            candidates.push(home.join(relative));
        }

        #[cfg(target_os = "macos")]
        {
            candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));
            candidates.push(PathBuf::from("/usr/local/bin/claude"));
        }

        #[cfg(target_os = "linux")]
        candidates.push(PathBuf::from("/usr/local/bin/claude"));
    }

    candidates
}

fn claude_binary_candidates() -> Vec<PathBuf> {
    claude_binary_candidates_for_home(&crate::config::get_home_dir())
}

fn run_fixed_claude_command(
    args: &'static [&'static str],
    timeout: Duration,
) -> Result<ClaudeAuthCommandOutput, AppError> {
    let mut child = claude_binary_candidates()
        .into_iter()
        .find_map(|binary| {
            match Command::new(binary)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => Some(Ok(child)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => Some(Err(command_error("claude_cli_status_failed"))),
            }
        })
        .unwrap_or_else(|| Err(command_error("claude_cli_not_installed")))?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(command_error("claude_cli_timeout"));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(command_error("claude_cli_status_failed"));
            }
        }
    };
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| command_error("claude_cli_status_failed"))?
        .read_to_end(&mut stdout)
        .map_err(|_| command_error("claude_cli_status_failed"))?;
    Ok(ClaudeAuthCommandOutput {
        exit_code: status.code(),
        stdout,
    })
}

fn spawn_fixed_command(
    args: &'static [&'static str],
    timeout: Duration,
) -> ClaudeAuthCommandFuture<'static> {
    Box::pin(async move {
        tokio::task::spawn_blocking(move || run_fixed_claude_command(args, timeout))
            .await
            .map_err(|_| command_error("claude_cli_status_failed"))?
    })
}

impl ClaudeAuthCommandRunner for ProductionClaudeAuthCommandRunner {
    fn status(&self) -> ClaudeAuthCommandFuture<'_> {
        spawn_fixed_command(&CLAUDE_STATUS_ARGS, STATUS_TIMEOUT)
    }

    fn launch_login(&self) -> Result<(), AppError> {
        crate::commands::launch_terminal_running(CLAUDE_LOGIN_COMMAND_LINE, "claude_auth_login")
            .map_err(|_| command_error("claude_cli_login_failed"))
    }

    fn logout(&self) -> ClaudeAuthCommandFuture<'_> {
        spawn_fixed_command(&CLAUDE_LOGOUT_ARGS, LOGOUT_TIMEOUT)
    }
}

pub struct ClaudeCliAuthService {
    runner: Arc<dyn ClaudeAuthCommandRunner>,
}

impl ClaudeCliAuthService {
    pub fn production() -> Self {
        Self::new(Arc::new(ProductionClaudeAuthCommandRunner))
    }

    pub fn new(runner: Arc<dyn ClaudeAuthCommandRunner>) -> Self {
        Self { runner }
    }

    fn disconnected(error_code: Option<String>, installed: bool) -> ClaudeCliAuthStatus {
        ClaudeCliAuthStatus {
            installed,
            authenticated: false,
            auth_method: None,
            subscription_type: None,
            quota_availability: "unavailable",
            error_code,
        }
    }

    fn parse_status(output: ClaudeAuthCommandOutput) -> ClaudeCliAuthStatus {
        if !matches!(output.exit_code, Some(0 | 1)) {
            return Self::disconnected(Some("claude_cli_status_failed".to_string()), true);
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
            return Self::disconnected(Some("claude_cli_status_malformed".to_string()), true);
        };
        let Some(object) = value.as_object() else {
            return Self::disconnected(Some("claude_cli_status_malformed".to_string()), true);
        };
        if object
            .get("loggedIn")
            .is_some_and(|logged_in| !logged_in.is_boolean())
        {
            return Self::disconnected(Some("claude_cli_status_malformed".to_string()), true);
        }
        if output.exit_code == Some(1) {
            return Self::disconnected(None, true);
        }
        let subscription_type = match object
            .get("subscriptionType")
            .and_then(|value| value.as_str())
        {
            Some("pro") => Some(ClaudeSubscriptionType::Pro),
            Some("max") => Some(ClaudeSubscriptionType::Max),
            _ => None,
        };
        let auth_method = match object.get("authMethod").and_then(|value| value.as_str()) {
            Some("api_key") => Some(ClaudeAuthMethod::ApiKey),
            Some("claude.ai") => Some(ClaudeAuthMethod::ClaudeAccount),
            Some(_) => Some(ClaudeAuthMethod::Other),
            None => None,
        };
        ClaudeCliAuthStatus {
            installed: true,
            authenticated: true,
            auth_method,
            subscription_type,
            quota_availability: "unavailable",
            error_code: None,
        }
    }

    pub async fn status(&self) -> ClaudeCliAuthStatus {
        match self.runner.status().await {
            Ok(output) => Self::parse_status(output),
            Err(AppError::Message(code)) => Self::disconnected(
                Some(code.clone()),
                code.as_str() != "claude_cli_not_installed",
            ),
            Err(_) => Self::disconnected(Some("claude_cli_status_failed".to_string()), true),
        }
    }

    pub fn start_login(&self) -> Result<(), AppError> {
        self.runner.launch_login()
    }

    pub async fn logout(&self) -> ClaudeCliAuthStatus {
        let logout = self.runner.logout().await;
        let mut status = self.status().await;
        match logout {
            Ok(output) if output.exit_code == Some(0) => status,
            Ok(_) => {
                status.error_code = Some("claude_cli_logout_failed".to_string());
                status
            }
            Err(AppError::Message(code)) => {
                status.error_code = Some(code);
                status
            }
            Err(_) => {
                status.error_code = Some("claude_cli_logout_failed".to_string());
                status
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct MockRunner {
        statuses: Mutex<VecDeque<Result<ClaudeAuthCommandOutput, AppError>>>,
        logout: Mutex<Option<Result<ClaudeAuthCommandOutput, AppError>>>,
        login_calls: AtomicUsize,
        logout_calls: AtomicUsize,
    }

    impl MockRunner {
        fn with_statuses(statuses: Vec<Result<ClaudeAuthCommandOutput, AppError>>) -> Self {
            Self {
                statuses: Mutex::new(statuses.into()),
                logout: Mutex::new(None),
                login_calls: AtomicUsize::new(0),
                logout_calls: AtomicUsize::new(0),
            }
        }
    }

    impl ClaudeAuthCommandRunner for MockRunner {
        fn status(&self) -> ClaudeAuthCommandFuture<'_> {
            let output = self.statuses.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { output })
        }

        fn launch_login(&self) -> Result<(), AppError> {
            self.login_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn logout(&self) -> ClaudeAuthCommandFuture<'_> {
            self.logout_calls.fetch_add(1, Ordering::SeqCst);
            let output = self.logout.lock().unwrap().take().unwrap();
            Box::pin(async move { output })
        }
    }

    fn output(exit_code: i32, stdout: &str) -> Result<ClaudeAuthCommandOutput, AppError> {
        Ok(ClaudeAuthCommandOutput {
            exit_code: Some(exit_code),
            stdout: stdout.as_bytes().to_vec(),
        })
    }

    #[tokio::test]
    async fn status_maps_official_exit_codes_and_allowlisted_json_only() {
        for (stdout, subscription_type) in [
            (
                r#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"pro","token":"must-not-escape"}"#,
                Some(ClaudeSubscriptionType::Pro),
            ),
            (
                r#"{"loggedIn":false,"subscriptionType":"max","unknown":{"secret":"ignored"}}"#,
                Some(ClaudeSubscriptionType::Max),
            ),
            (r#"{}"#, None),
        ] {
            let runner = Arc::new(MockRunner::with_statuses(vec![output(0, stdout)]));
            let status = ClaudeCliAuthService::new(runner).status().await;
            assert!(status.installed);
            assert!(status.authenticated, "exit 0 is authoritative");
            assert_eq!(status.subscription_type, subscription_type);
            assert_eq!(status.quota_availability, "unavailable");
            assert_eq!(status.error_code, None);
            let json = serde_json::to_string(&status).unwrap();
            assert!(!json.contains("must-not-escape"));
            assert!(!json.contains("ignored"));
            assert!(!json.contains("token"));
        }

        let runner = Arc::new(MockRunner::with_statuses(vec![output(
            0,
            r#"{"loggedIn":true,"authMethod":"api_key","apiKeySource":"ANTHROPIC_API_KEY"}"#,
        )]));
        let status = ClaudeCliAuthService::new(runner).status().await;
        assert_eq!(status.auth_method, Some(ClaudeAuthMethod::ApiKey));
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("apiKeySource"));

        let runner = Arc::new(MockRunner::with_statuses(vec![output(
            1,
            r#"{"loggedIn":false}"#,
        )]));
        let status = ClaudeCliAuthService::new(runner).status().await;
        assert!(status.installed);
        assert!(!status.authenticated);
        assert_eq!(status.error_code, None);
    }

    #[test]
    fn gui_path_fallback_includes_native_claude_install() {
        let candidates = claude_binary_candidates_for_home(Path::new("/Users/example"));
        assert_eq!(candidates.first(), Some(&PathBuf::from("claude")));
        #[cfg(not(target_os = "windows"))]
        assert!(candidates.contains(&PathBuf::from("/Users/example/.local/bin/claude")));
    }

    #[tokio::test]
    async fn status_fails_closed_for_absent_timeout_malformed_and_command_failure() {
        for (error, installed, code) in [
            (
                "claude_cli_not_installed",
                false,
                "claude_cli_not_installed",
            ),
            ("claude_cli_timeout", true, "claude_cli_timeout"),
            ("claude_cli_status_failed", true, "claude_cli_status_failed"),
        ] {
            let runner = Arc::new(MockRunner::with_statuses(vec![Err(AppError::Message(
                error.to_string(),
            ))]));
            let status = ClaudeCliAuthService::new(runner).status().await;
            assert_eq!(status.installed, installed);
            assert!(!status.authenticated);
            assert_eq!(status.error_code.as_deref(), Some(code));
            assert_eq!(status.quota_availability, "unavailable");
        }

        for output in [output(0, "not-json"), output(0, "[]"), output(2, "{}")] {
            let runner = Arc::new(MockRunner::with_statuses(vec![output]));
            let status = ClaudeCliAuthService::new(runner).status().await;
            assert!(!status.authenticated);
            assert!(status.error_code.is_some());
        }
    }

    #[tokio::test]
    async fn login_and_logout_use_only_official_auth_commands_and_logout_refreshes() {
        assert_eq!(CLAUDE_STATUS_ARGS, ["auth", "status"]);
        assert_eq!(CLAUDE_LOGIN_COMMAND_LINE, "claude auth login");
        assert_eq!(CLAUDE_LOGOUT_ARGS, ["auth", "logout"]);
        for forbidden in [
            ["setup", "token"].join("-"),
            [".credentials", ".json"].concat(),
            [".claude", ".json"].concat(),
            ["Key", "chain"].concat(),
        ] {
            let commands = format!(
                "{} {} {}",
                CLAUDE_STATUS_ARGS.join(" "),
                CLAUDE_LOGIN_COMMAND_LINE,
                CLAUDE_LOGOUT_ARGS.join(" ")
            );
            assert!(!commands.contains(&forbidden));
        }

        let runner = Arc::new(MockRunner::with_statuses(vec![output(
            1,
            r#"{"loggedIn":false}"#,
        )]));
        *runner.logout.lock().unwrap() = Some(output(0, "{}"));
        let service = ClaudeCliAuthService::new(runner.clone());
        service.start_login().unwrap();
        let status = service.logout().await;
        assert_eq!(runner.login_calls.load(Ordering::SeqCst), 1);
        assert_eq!(runner.logout_calls.load(Ordering::SeqCst), 1);
        assert!(status.installed);
        assert!(!status.authenticated);
        assert_eq!(status.quota_availability, "unavailable");
    }
}
