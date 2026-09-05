//! Read-only provider access for controls while a turn owns the Agent mutex.
//!
//! Entries follow the exact Agent allocation, rather than the connection's
//! initial provider or a process-wide active account. Weak references neither
//! retain sessions/credentials nor permit allocator-address reuse to match a
//! different agent. Normal model/auth switches update the same provider handle
//! in place; strict role restoration registers its replacement before releasing
//! the Agent lock.

use crate::agent::Agent;
use crate::provider::Provider;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex as StdMutex, Weak};
use tokio::sync::Mutex;

struct Entry {
    agent: Weak<Mutex<Agent>>,
    provider: Weak<dyn Provider>,
}

static PROVIDERS: LazyLock<StdMutex<HashMap<usize, Entry>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

pub(super) fn shared_agent(agent: Agent) -> Arc<Mutex<Agent>> {
    let provider = agent.provider_handle();
    let agent = Arc::new(Mutex::new(agent));
    register(&agent, &provider);
    agent
}

/// Call while holding the Agent lock if its instance is replaced in place.
pub(super) fn register(agent: &Arc<Mutex<Agent>>, provider: &Arc<dyn Provider>) {
    let mut providers = PROVIDERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    providers.retain(|_, entry| entry.agent.strong_count() > 0);
    providers.insert(
        Arc::as_ptr(agent) as usize,
        Entry {
            agent: Arc::downgrade(agent),
            provider: Arc::downgrade(provider),
        },
    );
}

/// Access is for catalog inspection or private forks, never primary mutations.
pub(super) fn for_agent(agent: &Arc<Mutex<Agent>>) -> Option<Arc<dyn Provider>> {
    if let Ok(guard) = agent.try_lock() {
        let provider = guard.provider_handle();
        register(agent, &provider);
        return Some(provider);
    }
    let providers = PROVIDERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = providers.get(&(Arc::as_ptr(agent) as usize))?;
    let registered = entry.agent.upgrade()?;
    Arc::ptr_eq(agent, &registered)
        .then(|| entry.provider.upgrade())
        .flatten()
}
