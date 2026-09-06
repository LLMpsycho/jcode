use super::{clone_split_session, create_transfer_child_session};
use crate::message::{ContentBlock, Role};
use crate::session::Session;

struct HomeGuard(Option<std::ffi::OsString>);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match &self.0 {
            Some(previous) => crate::env::set_var("JCODE_HOME", previous),
            None => crate::env::remove_var("JCODE_HOME"),
        }
    }
}

fn verify_child_routing(transfer: bool) {
    let _lock = crate::storage::lock_test_env();
    let temporary = tempfile::tempdir().expect("temporary home");
    let _home = HomeGuard(std::env::var_os("JCODE_HOME"));
    crate::env::set_var("JCODE_HOME", temporary.path());
    for explicit in [false, true] {
        for visible_history in [false, true] {
            let mut parent = Session::create(None, Some("fork parent".into()));
            parent.model = Some("chosen-model".into());
            parent.provider_key = Some("openai-oauth".into());
            parent.route_api_method = Some("openai-oauth".into());
            parent.reasoning_effort = Some("high".into());
            parent.role_model_selection = explicit.then(|| crate::config::ConfigModelRoute {
                model: "chosen-model".into(),
                api_method: "openai-oauth".into(),
                provider_label: "selected-account".into(),
            });
            parent.subagent_model = Some("worker-model".into());
            parent.autoreview_enabled = Some(true);
            parent.autojudge_enabled = Some(false);
            parent.provider_session_id = Some("parent-provider-conversation".into());
            if visible_history {
                parent.add_message(
                    Role::User,
                    vec![ContentBlock::Text {
                        text: "Inspect this project".into(),
                        cache_control: None,
                    }],
                );
            }
            parent.save().expect("save parent");
            let (id, _) = if transfer {
                create_transfer_child_session(&parent.id, &parent, None)
            } else {
                clone_split_session(&parent.id, None)
            }
            .expect("create child");
            let mut child = Session::load(&id).expect("child is durable before launch");
            assert_eq!(child.parent_id.as_deref(), Some(parent.id.as_str()));
            assert_ne!(child.id, parent.id);
            assert_eq!(child.model, parent.model);
            assert_eq!(child.provider_key, parent.provider_key);
            assert_eq!(child.route_api_method, parent.route_api_method);
            assert_eq!(child.reasoning_effort, parent.reasoning_effort);
            assert_eq!(child.role_model_selection, parent.role_model_selection);
            assert_eq!(child.subagent_model, parent.subagent_model);
            assert_eq!(child.autoreview_enabled, parent.autoreview_enabled);
            assert_eq!(child.autojudge_enabled, parent.autojudge_enabled);
            assert!(child.provider_session_id.is_none());
            child.reasoning_effort = Some("low".into());
            child.role_model_selection = None;
            child.save().expect("save independent child");
            let original = Session::load(&parent.id).expect("reload parent");
            assert_eq!(original.reasoning_effort, parent.reasoning_effort);
            assert_eq!(original.role_model_selection, parent.role_model_selection);
            assert_eq!(original.provider_session_id, parent.provider_session_id);
        }
    }
}

#[test]
fn post_merge_fork_remote_split_persists_empty_child_and_preserves_route() {
    verify_child_routing(false);
}

#[test]
fn post_merge_fork_transfer_persists_empty_child_and_preserves_route() {
    verify_child_routing(true);
}
