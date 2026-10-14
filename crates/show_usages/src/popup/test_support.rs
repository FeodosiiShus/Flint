use gpui::{Context, SharedString, Window};
use project::Location;

use crate::model::{DisplayRow, UsageFilters, UsageRow, UsageScope};

use super::ShowUsagesPopup;

impl ShowUsagesPopup {
    pub(crate) fn display_rows_for_test(&self) -> Vec<DisplayRow> {
        self.model
            .as_ref()
            .map(|model| model.display_rows().to_vec())
            .unwrap_or_default()
    }

    pub(crate) fn usage_rows_for_test(&self) -> Vec<UsageRow<Location>> {
        let Some(model) = self.model.as_ref() else {
            return Vec::new();
        };
        (0..model.display_rows().len())
            .filter_map(|display_index| model.usage_row_at(display_index).cloned())
            .collect()
    }

    pub(crate) fn selected_for_test(&self) -> Option<usize> {
        self.selected
    }

    pub(crate) fn status_for_test(&self) -> Option<SharedString> {
        self.model
            .as_ref()
            .map(|model| model.status_text(self.is_searching()))
    }

    pub(crate) fn query_for_test(&self) -> &str {
        &self.query
    }

    pub(crate) fn filters_for_test(&self) -> UsageFilters {
        self.state.filters
    }

    pub(crate) fn choose_scope_for_test(
        &mut self,
        scope: UsageScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_scope(scope, window, cx);
    }
}
