use gpui::{Pixels, SharedString, px};

pub(crate) const LOG_TAB_NAME: &str = "Log";
pub(crate) const CONSOLE_TAB_NAME: &str = "Console";

const LOG_TAB_ORDER_WEIGHT: u32 = 50;
pub(crate) const CONSOLE_TAB_ORDER_WEIGHT: u32 = 60;
const OTHER_TAB_ORDER_WEIGHT: u32 = 100;
const DROPPED_TAB_GAP: Pixels = px(1.);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ContentTabKey {
    Log(SharedString),
    Console,
}

pub(crate) fn order_weight_for_tab_name(tab_name: &str) -> u32 {
    match tab_name {
        LOG_TAB_NAME => LOG_TAB_ORDER_WEIGHT,
        CONSOLE_TAB_NAME => CONSOLE_TAB_ORDER_WEIGHT,
        _ => OTHER_TAB_ORDER_WEIGHT,
    }
}

#[derive(Clone, Debug)]
struct ContentTabEntry {
    key: ContentTabKey,
    order_weight: u32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ContentTabs {
    entries: Vec<ContentTabEntry>,
    selected: Option<ContentTabKey>,
    selection_history: Vec<ContentTabKey>,
}

impl ContentTabs {
    pub(crate) fn keys(&self) -> impl Iterator<Item = &ContentTabKey> {
        self.entries.iter().map(|entry| &entry.key)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn contains(&self, key: &ContentTabKey) -> bool {
        self.index_of(key).is_some()
    }

    pub(crate) fn index_of(&self, key: &ContentTabKey) -> Option<usize> {
        self.entries.iter().position(|entry| &entry.key == key)
    }

    pub(crate) fn selected(&self) -> Option<&ContentTabKey> {
        self.selected.as_ref()
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected.as_ref().and_then(|key| self.index_of(key))
    }

    pub(crate) fn selection_history(&self) -> &[ContentTabKey] {
        &self.selection_history
    }

    pub(crate) fn append(&mut self, key: ContentTabKey, order_weight: u32) {
        if self.contains(&key) {
            return;
        }
        self.entries.push(ContentTabEntry { key, order_weight });
    }

    pub(crate) fn insert_by_weight(&mut self, key: ContentTabKey, order_weight: u32) {
        if self.contains(&key) {
            return;
        }
        let index = self
            .entries
            .iter()
            .position(|entry| entry.order_weight > order_weight)
            .unwrap_or(self.entries.len());
        self.entries
            .insert(index, ContentTabEntry { key, order_weight });
    }

    pub(crate) fn select(&mut self, key: &ContentTabKey) -> bool {
        if !self.contains(key) {
            return false;
        }
        self.selection_history.retain(|entry| entry != key);
        self.selection_history.insert(0, key.clone());
        let changed = self.selected.as_ref() != Some(key);
        self.selected = Some(key.clone());
        changed
    }

    pub(crate) fn remove(&mut self, key: &ContentTabKey) -> bool {
        let Some(index_to_remove) = self.index_of(key) else {
            return false;
        };
        let selected_index = self.selected_index();
        let was_selected = self.selected.as_ref() == Some(key);
        if was_selected {
            self.selected = None;
        }
        let index_to_select = if was_selected {
            if index_to_remove >= 1 {
                Some(index_to_remove - 1)
            } else if self.entries.len() > 1 {
                Some(0)
            } else {
                None
            }
        } else {
            selected_index
                .filter(|selected_index| *selected_index > index_to_remove)
                .map(|selected_index| selected_index - 1)
        };

        self.selection_history.retain(|entry| entry != key);
        self.entries.remove(index_to_remove);

        if self.entries.is_empty() {
            self.selected = None;
            return true;
        }
        if let Some(index_to_select) = index_to_select {
            let to_select = self.selection_history.first().cloned().or_else(|| {
                self.entries
                    .get(index_to_select)
                    .map(|entry| entry.key.clone())
            });
            if let Some(to_select) = to_select
                && self.selected.as_ref() != Some(&to_select)
            {
                self.select(&to_select);
            }
        }
        true
    }

    pub(crate) fn next_key(&self) -> Option<ContentTabKey> {
        self.neighbour_key(1)
    }

    pub(crate) fn previous_key(&self) -> Option<ContentTabKey> {
        self.neighbour_key(self.entries.len().saturating_sub(1))
    }

    fn neighbour_key(&self, offset: usize) -> Option<ContentTabKey> {
        let count = self.entries.len();
        if count <= 1 {
            return None;
        }
        let index = match self.selected_index() {
            Some(index) => (index + offset) % count,
            None if offset == 1 => 0,
            None => count - 1,
        };
        self.entries.get(index).map(|entry| entry.key.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TabSlot {
    Full,
    Squeezed(Pixels),
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabStripLayout {
    pub slots: Vec<TabSlot>,
    pub has_dropped_tabs: bool,
}

impl TabStripLayout {
    pub(crate) fn hidden_indices(&self) -> Vec<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| **slot == TabSlot::Hidden)
            .map(|(index, _)| index)
            .collect()
    }
}

pub(crate) fn layout_tab_strip(
    widths: &[Pixels],
    selected: Option<usize>,
    available_width: Option<Pixels>,
    toolbar_width: Pixels,
) -> TabStripLayout {
    let Some(available_width) = available_width else {
        return TabStripLayout {
            slots: vec![TabSlot::Full; widths.len()],
            has_dropped_tabs: false,
        };
    };
    let selected = selected.unwrap_or(0);
    let to_fit_width = available_width - toolbar_width;
    let mut required_width = widths.iter().fold(px(0.), |sum, width| sum + *width);
    let mut to_layout: Vec<usize> = (0..widths.len()).collect();
    let mut dropped = Vec::new();

    while required_width > to_fit_width && to_layout.len() > 1 {
        let (Some(first), Some(last)) = (to_layout.first().copied(), to_layout.last().copied())
        else {
            break;
        };
        let label_to_drop = if first != selected {
            first
        } else if last != selected {
            last
        } else {
            break;
        };
        let dropped_width = widths.get(label_to_drop).copied().unwrap_or(px(0.));
        required_width = required_width - (dropped_width + DROPPED_TAB_GAP);
        dropped.push(label_to_drop);
        to_layout.retain(|index| *index != label_to_drop);
    }

    let mut slots = vec![TabSlot::Hidden; widths.len()];
    let mut each_x = px(0.);
    let mut reached_bounds = false;
    for index in to_layout {
        let width = widths.get(index).copied().unwrap_or(px(0.));
        let Some(slot) = slots.get_mut(index) else {
            continue;
        };
        if each_x + width < to_fit_width {
            *slot = TabSlot::Full;
            each_x = each_x + width;
        } else if !reached_bounds {
            let squeezed_width = (available_width - each_x - toolbar_width).max(px(0.));
            *slot = TabSlot::Squeezed(squeezed_width);
            each_x = each_x + squeezed_width;
            reached_bounds = true;
        }
    }

    TabStripLayout {
        slots,
        has_dropped_tabs: !dropped.is_empty(),
    }
}
