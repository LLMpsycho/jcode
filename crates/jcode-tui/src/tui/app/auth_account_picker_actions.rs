//! Translate account overlay keys into account actions.
use super::*;

impl App {
    pub(crate) fn next_account_picker_action(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> anyhow::Result<Option<crate::tui::account_picker::AccountPickerCommand>> {
        use crate::tui::account_picker::OverlayAction;

        let action = {
            let Some(picker_cell) = self.account_picker_overlay.as_ref() else {
                return Ok(None);
            };
            let mut picker = picker_cell.borrow_mut();
            picker.handle_overlay_key(code, modifiers)?
        };

        match action {
            OverlayAction::Continue => Ok(None),
            OverlayAction::Close => {
                self.account_picker_overlay = None;
                Ok(None)
            }
            OverlayAction::Execute(command) => {
                self.account_picker_overlay = None;
                Ok(Some(command))
            }
        }
    }
}
