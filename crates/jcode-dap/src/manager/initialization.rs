use std::sync::{Arc, Mutex};

use super::{DebugSessionManager, ManagerCore, ManagerInitialization, Registry};
use crate::session::next_manager_id;
use crate::{DebugOperationConfig, DebugSessionManagerConfig, Result};

impl ManagerInitialization for DebugSessionManager {
    fn initialize(
        config: DebugSessionManagerConfig,
        operations: DebugOperationConfig,
    ) -> Result<Self> {
        config.validate()?;
        operations.validate()?;
        Ok(Self {
            core: Arc::new(ManagerCore {
                config,
                operations: Arc::new(operations),
                manager_id: next_manager_id()?,
                registry: Mutex::new(Registry::default()),
            }),
        })
    }
}
