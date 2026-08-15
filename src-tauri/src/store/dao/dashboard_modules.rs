//! Compatibility entry points for the v15 dashboard-module command names.
//!
//! Runtime semantics are Agent-centric. The legacy `dashboard_modules` table is
//! intentionally frozen and is never read or written from this module.

use crate::error::AppError;
use crate::store::Database;
use crate::usage::domain::{AgentModuleInput, AgentModuleView};

impl Database {
    pub fn list_dashboard_modules(&self) -> Result<Vec<AgentModuleView>, AppError> {
        self.list_agent_modules()
    }

    pub fn save_dashboard_module(
        &self,
        input: &AgentModuleInput,
    ) -> Result<AgentModuleView, AppError> {
        self.save_agent_module(input)
    }

    pub fn reorder_dashboard_modules(
        &self,
        module_ids: &[String],
    ) -> Result<Vec<AgentModuleView>, AppError> {
        self.reorder_agent_modules(module_ids)
    }

    pub fn set_dashboard_module_visibility(
        &self,
        module_id: &str,
        visible: bool,
    ) -> Result<AgentModuleView, AppError> {
        self.set_agent_module_visibility(module_id, visible)
    }

    pub fn delete_dashboard_module(&self, module_id: &str) -> Result<(), AppError> {
        self.delete_agent_module(module_id).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_entry_points_use_agent_rows_only() {
        let db = Database::memory().unwrap();
        let defaults = db.list_dashboard_modules().unwrap();
        assert_eq!(defaults.len(), 5);
        assert!(defaults.iter().all(|module| module.is_fixed));
        assert!(!defaults.iter().any(|module| module.id == "api"));

        let custom = db
            .save_dashboard_module(&AgentModuleInput {
                id: None,
                name: "Custom Agent".to_string(),
                sort_order: 5,
                visible: true,
            })
            .unwrap();
        db.delete_dashboard_module(&custom.id).unwrap();
        assert_eq!(db.list_dashboard_modules().unwrap().len(), 5);
    }

    #[test]
    fn compatibility_save_rejects_the_legacy_api_identity() {
        let db = Database::memory().unwrap();
        let error = db
            .save_dashboard_module(&AgentModuleInput {
                id: Some("api".to_string()),
                name: "API".to_string(),
                sort_order: 5,
                visible: true,
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "invalid_agent_module");
    }
}
