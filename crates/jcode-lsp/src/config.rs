use sha2::{Digest, Sha256};

use crate::{LspConfig, LspError, Result};

pub fn config_digest(config: &LspConfig) -> Result<[u8; 32]> {
    let issues = config.validation_issues();
    if !issues.is_empty() {
        return Err(LspError::InvalidConfig(issues.join("; ")));
    }
    let bytes =
        serde_json::to_vec(config).map_err(|error| LspError::InvalidConfig(error.to_string()))?;
    Ok(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_deterministic_and_configuration_sensitive() {
        let first = LspConfig::default();
        let mut second = first.clone();
        assert_eq!(
            config_digest(&first).unwrap(),
            config_digest(&first).unwrap()
        );
        second.request_timeout_seconds += 1;
        assert_ne!(
            config_digest(&first).unwrap(),
            config_digest(&second).unwrap()
        );
    }

    #[test]
    fn invalid_configuration_is_rejected_before_digesting() {
        let config = LspConfig {
            request_timeout_seconds: 0,
            ..LspConfig::default()
        };
        assert!(matches!(
            config_digest(&config),
            Err(LspError::InvalidConfig(_))
        ));
    }
}
