use crate::fs_util::atomic_write;
use crate::platform::PlatformPaths;
use crate::policy::model::PolicyConfig;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum PolicyStoreError {
    #[error("cannot resolve home directory")]
    HomeNotFound,
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("toml parse/serialize error: {0}")]
    Toml(String),
    #[error("io error: {0}")]
    Io(String),
}

pub struct PolicyStore;

impl PolicyStore {
    pub fn policy_path() -> Result<PathBuf, PolicyStoreError> {
        let paths = PlatformPaths::detect().ok_or(PolicyStoreError::HomeNotFound)?;
        Ok(paths.policy_toml())
    }

    pub fn load_or_init() -> Result<PolicyConfig, PolicyStoreError> {
        let path = Self::policy_path()?;
        if !path.exists() {
            let default_cfg = PolicyConfig::default();
            Self::save(&default_cfg)?;
            return Ok(default_cfg);
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| PolicyStoreError::Io(format!("{}: {}", path.display(), e)))?;
        let mut cfg: PolicyConfig =
            toml::from_str(&raw).map_err(|e| PolicyStoreError::Toml(e.to_string()))?;
        Self::normalize_legacy(&mut cfg);
        Self::validate(&cfg)?;
        Ok(cfg)
    }

    pub(crate) fn normalize_legacy(cfg: &mut PolicyConfig) {
        if !cfg.budget.min_recall_score.is_finite() {
            cfg.budget.min_recall_score = 0.6;
        } else {
            cfg.budget.min_recall_score = cfg.budget.min_recall_score.clamp(0.0, 1.0);
        }
    }

    pub fn save(cfg: &PolicyConfig) -> Result<PathBuf, PolicyStoreError> {
        Self::validate(cfg)?;
        let path = Self::policy_path()?;
        let body =
            toml::to_string_pretty(cfg).map_err(|e| PolicyStoreError::Toml(e.to_string()))?;
        atomic_write(&path, body.as_bytes(), false)
            .map_err(|e| PolicyStoreError::Io(e.to_string()))?;
        Ok(path)
    }

    pub fn validate(cfg: &PolicyConfig) -> Result<(), PolicyStoreError> {
        if !cfg.routing.inbox_uri.starts_with("viking://") {
            return Err(PolicyStoreError::Validation(
                "routing.inbox_uri must start with viking://".into(),
            ));
        }
        if !(0.0..=1.0).contains(&cfg.budget.auto_expand_l1_threshold) {
            return Err(PolicyStoreError::Validation(
                "budget.auto_expand_l1_threshold must be between 0.0 and 1.0".into(),
            ));
        }
        if cfg.budget.max_recall_tokens == 0 {
            return Err(PolicyStoreError::Validation(
                "budget.max_recall_tokens must be greater than 0".into(),
            ));
        }
        let score = cfg.budget.min_recall_score;
        if !score.is_finite() || !(0.0..=1.0).contains(&score) {
            return Err(PolicyStoreError::Validation(
                "budget.min_recall_score must be finite and between 0.0 and 1.0".into(),
            ));
        }
        if !cfg.lifecycle.archival.archive_uri.starts_with("viking://") {
            return Err(PolicyStoreError::Validation(
                "lifecycle.archival.archive_uri must start with viking://".into(),
            ));
        }
        for mapping in &cfg.routing.workspace_mappings {
            if !mapping.resource_uri.starts_with("viking://") {
                return Err(PolicyStoreError::Validation(format!(
                    "workspace mapping '{}' must target a viking:// URI",
                    mapping.local_path.display()
                )));
            }
        }
        // Validate standard 5-field or 6-field cron expression
        if !cfg.lifecycle.dreaming.cron.trim().is_empty() {
            let expr = normalize_cron(&cfg.lifecycle.dreaming.cron);
            cron::Schedule::from_str(&expr).map_err(|e| {
                PolicyStoreError::Validation(format!(
                    "invalid lifecycle.dreaming.cron '{}': {}",
                    cfg.lifecycle.dreaming.cron, e
                ))
            })?;
        }
        Ok(())
    }
}

/// Convert a standard 5-field Unix cron (`0 3 * * *`) into the 6-field format expected by `cron` crate.
pub fn normalize_cron(raw: &str) -> String {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    if parts.len() == 5 {
        format!("0 {}", raw.trim())
    } else {
        raw.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy_validates() {
        let cfg = PolicyConfig::default();
        assert!(PolicyStore::validate(&cfg).is_ok());
    }

    #[test]
    fn test_invalid_inbox_rejected() {
        let mut cfg = PolicyConfig::default();
        cfg.routing.inbox_uri = "https://wrong".to_string();
        assert!(PolicyStore::validate(&cfg).is_err());
    }

    #[test]
    fn test_invalid_min_recall_score_rejected() {
        let mut cfg = PolicyConfig::default();
        for score in [1.1, -0.1, f32::NAN, f32::INFINITY] {
            cfg.budget.min_recall_score = score;
            let error = PolicyStore::validate(&cfg).unwrap_err();
            assert!(error.to_string().contains("budget.min_recall_score"));
        }
        for score in [0.0, 1.0] {
            cfg.budget.min_recall_score = score;
            assert!(PolicyStore::validate(&cfg).is_ok());
        }
    }

    #[test]
    fn test_legacy_min_recall_score_normalized_on_load() {
        let mut cfg = PolicyConfig::default();
        cfg.budget.min_recall_score = 1.4;
        PolicyStore::normalize_legacy(&mut cfg);
        assert_eq!(cfg.budget.min_recall_score, 1.0);
        assert!(PolicyStore::validate(&cfg).is_ok());

        cfg.budget.min_recall_score = -0.3;
        PolicyStore::normalize_legacy(&mut cfg);
        assert_eq!(cfg.budget.min_recall_score, 0.0);

        cfg.budget.min_recall_score = f32::NAN;
        PolicyStore::normalize_legacy(&mut cfg);
        assert_eq!(cfg.budget.min_recall_score, 0.6);
    }

    #[test]
    fn test_invalid_cron_rejected() {
        let mut cfg = PolicyConfig::default();
        cfg.lifecycle.dreaming.cron = "not a cron".to_string();
        assert!(PolicyStore::validate(&cfg).is_err());
    }
}
