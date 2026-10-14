use gpui::SharedString;

use super::UsagesHint;

impl UsagesHint {
    pub(crate) fn message_for_test(&self) -> SharedString {
        self.content.message.clone()
    }

    pub(crate) fn is_warning_for_test(&self) -> bool {
        self.content.warning
    }

    pub(crate) fn second_invocation_for_test(&self) -> Option<SharedString> {
        self.content.second_invocation.clone()
    }
}
