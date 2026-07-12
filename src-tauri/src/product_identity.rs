use std::path::PathBuf;

pub const DISPLAY_NAME: &str = "LLM Usage Bar";
pub const APP_SLUG: &str = "llm-usage-bar";
pub const DATABASE_FILE: &str = "llm-usage-bar.db";
pub const LOG_BASENAME: &str = "llm-usage-bar";
pub const BUNDLE_ID: &str = "com.llmusagebar.desktop";
pub const REPOSITORY: &str = "Xr810/LLM-Usage-Bar";
pub const LEGACY_DISPLAY_NAME: &str = "CC Switch";
pub const LEGACY_SLUG: &str = "cc-switch";
pub const LEGACY_DATA_DIR: &str = ".cc-switch";
pub const LEGACY_DATABASE_FILE: &str = "cc-switch.db";
/// Database filename migration accepts the last release schema as its source
/// even after later application schema versions are introduced.
pub const DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION: i32 = 13;
pub const DATABASE_IDENTITY_ARCHIVE_FILE: &str = "cc-switch.db.pre-llm-usage-bar-v14";
pub const DATABASE_IDENTITY_LEASE_FILE: &str = ".llm-usage-bar-database-migration.lock";

pub fn current_database_path(app_dir: &std::path::Path) -> PathBuf {
    app_dir.join(DATABASE_FILE)
}

pub fn prior_current_app_database_path(app_dir: &std::path::Path) -> PathBuf {
    app_dir.join(LEGACY_DATABASE_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn current_and_legacy_identity_are_explicitly_separate() {
        assert_eq!(DISPLAY_NAME, "LLM Usage Bar");
        assert_eq!(APP_SLUG, "llm-usage-bar");
        assert_eq!(DATABASE_FILE, "llm-usage-bar.db");
        assert_eq!(LOG_BASENAME, "llm-usage-bar");
        assert_eq!(BUNDLE_ID, "com.llmusagebar.desktop");
        assert_eq!(REPOSITORY, "Xr810/LLM-Usage-Bar");
        assert_eq!(LEGACY_DISPLAY_NAME, "CC Switch");
        assert_eq!(LEGACY_SLUG, "cc-switch");
        assert_eq!(LEGACY_DATA_DIR, ".cc-switch");
        assert_eq!(LEGACY_DATABASE_FILE, "cc-switch.db");
        assert_eq!(DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION, 13);
        assert_eq!(
            DATABASE_IDENTITY_ARCHIVE_FILE,
            "cc-switch.db.pre-llm-usage-bar-v14"
        );
        assert_eq!(
            DATABASE_IDENTITY_LEASE_FILE,
            ".llm-usage-bar-database-migration.lock"
        );
        assert_ne!(APP_SLUG, LEGACY_SLUG);
    }

    #[test]
    fn database_paths_use_current_and_legacy_filenames() {
        let app_dir = Path::new("app-data");

        assert_eq!(
            current_database_path(app_dir),
            app_dir.join("llm-usage-bar.db")
        );
        assert_eq!(
            prior_current_app_database_path(app_dir),
            app_dir.join("cc-switch.db")
        );
    }
}
