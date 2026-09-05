from pathlib import Path
import re
root = Path('crates/jcode-tui/src/tui/app')
def replace(path, old, new, count=1):
    path=Path(path)
    text=path.read_text()
    assert text.count(old)==count, (str(path), old[:80], text.count(old))
    path.write_text(text.replace(old,new))

replace(root/'tests/post_merge_review_launch.rs', 'use super::*;', 'use super::*;\nuse crate::protocol::ServerEvent;\nuse crate::tui::backend::RemoteConnection;')
replace(root/'remote/review_launch.rs', 'It will not be replayed automatically.".into(),', 'It will not be replayed automatically.",')

# Every split entry point shares the same pending metadata; do not allow a
# workspace/plain fork to steal a review's role, prompt or parent selection.
replace(root/'remote/workspace.rs', '    if let Some(target) = target {', '''    if let Some(target) = target {
        if super::super::commands_review::review_split_pending(app) {
            app.push_display_message(DisplayMessage::system("A session launch is already pending."));
            return Ok(true);
        }''')
replace(root/'remote/key_handling.rs', '                if trimmed == "/fork" || trimmed == "/split" {', '''                if trimmed == "/fork" || trimmed == "/split" {
                    if app_mod::commands_review::review_split_pending(app) {
                        app.push_display_message(DisplayMessage::system("A session launch is already pending."));
                        return Ok(());
                    }''')
# Record direct split request IDs too, without changing the old ability to
# fork the current state while a main-model turn is active.
replace(root/'remote/key_handling.rs', '                    remote.split().await?;', '''                    app.pending_split_label = Some("Split".to_string());
                    match remote.split().await {
                        Ok(id) => app.pending_split_request_id = Some(id),
                        Err(error) => {
                            super::review_launch::clear_launch(app);
                            return Err(error);
                        }
                    }''')
replace(root/'remote/workspace.rs', '            remote.split().await?;', '''            match remote.split().await {
                Ok(id) => app.pending_split_request_id = Some(id),
                Err(error) => {
                    super::review_launch::clear_launch(app);
                    return Err(error);
                }
            }''')
replace(root/'remote/input_dispatch.rs', 'if let Err(error) = remote.split().await {', '''let split_result = remote.split().await;
        if let Ok(id) = &split_result {
            app.pending_split_request_id = Some(*id);
        }
        if let Err(error) = split_result {''', count=2)
replace(root/'remote/review_launch.rs', 'fn clear_launch(app: &mut App) {', '''pub(super) fn clear_launch(app: &mut App) {
    app.workspace_client.cancel_pending_split();''')
replace(root.parent/'workspace_client.rs', '    pub(crate) fn queue_split_target(&mut self, target: WorkspaceSplitTarget) {', '''    pub(crate) fn cancel_pending_split(&mut self) {
        self.pending_split_target = None;
    }

    pub(crate) fn queue_split_target(&mut self, target: WorkspaceSplitTarget) {''')
replace(root/'remote/review_controls.rs', '    if let Err(error) = split_result {', '    if let Err(error) = split_result {\n        app.workspace_client.cancel_pending_split();')
# Cover all neighboring command entry points with the same existing collision
# fixture, rather than creating another almost-identical test class.
replace(root/'tests/post_merge_review_launch.rs', '            app.input = "/transfer".into();', '            for command in ["/transfer", "/split", "/fork", "/workspace add", "/workspace add up"] {\n            app.input = command.into();')
replace(root/'tests/post_merge_review_launch.rs', '            assert!(!app.pending_transfer_request);', '            assert!(!app.pending_transfer_request);\n            }')

# Move cohesive pre-existing code out of oversized modules. Do not edit size
# baselines, add allow attributes, or condense code to evade the ratchet.
path=root.parent/'app.rs'
s=path.read_text()
a=s.index('#[derive(Debug, Clone)]\nstruct PendingRemoteMessage')
b=s.index('/// A reasoning trace anchored',a)
block=s[a:b]
block=re.sub(r'^struct ', 'pub(super) struct ',block,flags=re.M)
block=re.sub(r'^    ([a-z_]+):',r'    pub(super) \1:',block,flags=re.M)
(root/'pending_requests.rs').write_text('use super::PreparedTransferSession;\nuse std::sync::mpsc;\nuse std::time::Instant;\n\n'+block)
s=s[:a]+s[b:]
s=s.replace('mod commands_review;', 'mod commands_review;\nmod pending_requests;\nuse pending_requests::{PendingLocalTransfer, PendingRemoteMessage, PendingSplitPrompt};')
path.write_text(s)

path=root/'tui_lifecycle.rs'
s=path.read_text()
a=s.index('    pub(super) fn apply_restored_reload_input')
b=s.index('    /// Re-parse keybinding snapshots',a)
block=s[a:b]
(root/'tui_reload_restore.rs').write_text('use super::{App, Instant, ProcessingStatus};\nuse super::state_ui::RestoredReloadInput;\n\nimpl App {\n'+block+'}\n')
s=s[:a]+s[b:]
s=s.replace('use super::state_ui::RestoredReloadInput;\n','')
path.write_text(s)
replace(root.parent/'app.rs','mod tui_lifecycle;','mod tui_lifecycle;\nmod tui_reload_restore;')

path=root/'remote/server_events.rs'
s=path.read_text()
a=s.index('        ServerEvent::SplitResponse {')
b=s.index('        ServerEvent::CompactResult {',a)
arm=s[a:b]
body=arm.split('        } => {\n',1)[1]
assert body.endswith('        }\n')
body=body[:-10]
(root/'remote/split_response.rs').write_text('''//! Finish the shared split handshake and publish its prepared child session.
use super::{App, DisplayMessage, finish_remote_split_launch, spawn_in_new_terminal};
use crate::tui::app as app_mod;

pub(super) fn handle_split_response(app: &mut App, id: u64, new_session_id: String, new_session_name: String) -> bool {
'''+body+'}\n')
s=s[:a]+'''        ServerEvent::SplitResponse { id, new_session_id, new_session_name, .. } =>
            super::split_response::handle_split_response(app, id, new_session_id, new_session_name),
'''+s[b:]
path.write_text(s)
replace(root/'remote.rs','mod server_events;', 'mod server_events;\nmod split_response;')

path=root/'tests.rs'
s=path.read_text()
a=s.index('fn seed_stale_clear_usage(')
b=s.index('#[path = "tests/role_review_launch.rs"]',a)
(root/'tests/clear_session_state.rs').write_text(s[a:b])
s=s[:a]+'include!("tests/clear_session_state.rs");\n\n'+s[b:]
path.write_text(s)

for p in [root/"pending_requests.rs", root/"tests/clear_session_state.rs"]:
    p.write_text(p.read_text().rstrip()+"\n")
