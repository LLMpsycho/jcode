use std::sync::{Arc, Mutex};

use super::{DebugSessionManager, ManagerCore, ManagerInitialization, Registry};
use crate::session::next_manager_id;
use crate::{DebugInspectionConfig, DebugOperationConfig, DebugSessionManagerConfig, Result};

impl ManagerInitialization for DebugSessionManager {
    fn initialize(
        config: DebugSessionManagerConfig,
        operations: DebugOperationConfig,
        inspection: DebugInspectionConfig,
    ) -> Result<Self> {
        config.validate()?;
        operations.validate()?;
        inspection.validate()?;
        Ok(Self {
            core: Arc::new(ManagerCore {
                config,
                operations: Arc::new(operations),
                inspection: Arc::new(inspection),
                manager_id: next_manager_id()?,
                registry: Mutex::new(Registry::default()),
            }),
        })
    }
}

#[cfg(test)]
mod phase30f_contract_tests {
    use super::*;
    use crate::DapError;

    fn manager_config() -> DebugSessionManagerConfig {
        DebugSessionManagerConfig::default()
    }

    #[test]
    fn existing_constructors_use_default_inspection_config() {
        let expected = DebugInspectionConfig::default();
        let direct = DebugSessionManager::new(manager_config()).unwrap();
        let with_operations = DebugSessionManager::new_with_operation_config(
            manager_config(),
            DebugOperationConfig::default(),
        )
        .unwrap();
        assert_eq!(
            direct.core.inspection.max_stack_frames_per_response,
            expected.max_stack_frames_per_response
        );
        assert_eq!(
            with_operations.core.inspection.max_variables_per_response,
            expected.max_variables_per_response
        );
    }

    #[test]
    fn additive_inspection_constructor_uses_shared_initialization() {
        let inspection = DebugInspectionConfig {
            max_stack_frames_per_response: 7,
            ..DebugInspectionConfig::default()
        };
        let manager = DebugSessionManager::new_with_operation_and_inspection_config(
            manager_config(),
            DebugOperationConfig::default(),
            inspection,
        )
        .unwrap();
        assert_eq!(manager.core.inspection.max_stack_frames_per_response, 7);
        assert_ne!(manager.core.manager_id, 0);
    }

    #[test]
    fn invalid_inspection_config_uses_existing_invalid_manager_configuration_error() {
        let inspection = DebugInspectionConfig {
            max_scopes_per_response: 0,
            ..DebugInspectionConfig::default()
        };
        assert!(matches!(
            DebugSessionManager::new_with_operation_and_inspection_config(
                manager_config(),
                DebugOperationConfig::default(),
                inspection,
            ),
            Err(DapError::InvalidManagerConfiguration { .. })
        ));
    }
}
