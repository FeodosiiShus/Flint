use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Action, Anchor, AnyElement, Context, DismissEvent, Entity, Focusable, Hsla, MouseButton,
    MouseDownEvent, MouseUpEvent, Pixels, Point, SharedString, Subscription, WeakEntity, Window,
    anchored, canvas, deferred, px, rgb, rgba,
};
use ui::{
    ContextMenu, ContextMenuEntry, IconButton, IconButtonShape, PopoverMenu, TextSize, Tooltip,
    prelude::*,
};
use util::ResultExt as _;

use super::content_tabs::{ContentTabKey, TabSlot, TabStripLayout, layout_tab_strip};
use super::{
    CloseActiveTab, GitToolWindow, HeaderSnapshot, HeaderTab, SelectNextTab, SelectPreviousTab,
    ShowListOfTabs, TabMenuState,
};

const TAB_HORIZONTAL_INSET: Pixels = px(12.);
const CLOSE_ICON_SIZE: Pixels = px(16.);
const CLOSE_ICON_TEXT_GAP: Pixels = px(8.);
const CLOSE_ICON_RIGHT_INSET: Pixels = px(7.);
const CLOSE_ICON_BOTTOM_SHIFT: Pixels = px(2.);
const CLOSE_ICONS_GAP: Pixels = px(3.);
const MAX_TAB_WIDTH: Pixels = px(400.);
const PILL_HORIZONTAL_OFFSET: Pixels = px(4.);
const PILL_FULL_HEIGHT: f32 = 28.;
const PILL_MIN_VERTICAL_OFFSET: f32 = 8.;
const PILL_RADIUS: Pixels = px(6.);
const UNDERLINE_HEIGHT: Pixels = px(3.);
const UNDERLINE_RADIUS: Pixels = px(2.);
const TOOLBAR_BUTTON_SIZE: Pixels = px(22.);
const CLOSE_TAB_TOOLTIP: &str = "Close tab";
const OPEN_LOG_TAB_TEXT: &str = "Open New Git Log Tab";
const OPEN_LOG_TAB_DESCRIPTION: &str = "Open new tab with Git Log";
const SHOW_HIDDEN_TABS_TEXT: &str = "Show Hidden Tabs";
const SHOW_HIDDEN_TABS_DESCRIPTION: &str =
    "Show dropdown with the list of tabs that don't fit on the screen in a single-row tab layout";
const WIDTH_CHANGE_TOLERANCE: f32 = 0.5;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TabDecoration {
    pub fill: Option<Hsla>,
    pub border: Option<Hsla>,
    pub hover_fill: Option<Hsla>,
    pub underline: Option<Hsla>,
}

pub(crate) fn tab_decoration(
    selected: bool,
    active: bool,
    light: bool,
    islands: bool,
) -> TabDecoration {
    let hover_fill: Hsla = match (islands, light) {
        (true, true) => rgba(0x00000012),
        (true, false) => rgba(0xFFFFFF17),
        (false, true) => rgb(0xEBECF0),
        (false, false) => rgb(0x393B40),
    }
    .into();
    if !islands {
        return if selected {
            let underline: Hsla = match (active, light) {
                (true, _) => rgb(0x3574F0),
                (false, true) => rgb(0xA8ADBD),
                (false, false) => rgb(0x5A5D63),
            }
            .into();
            TabDecoration {
                underline: Some(underline),
                ..TabDecoration::default()
            }
        } else {
            TabDecoration {
                hover_fill: Some(hover_fill),
                ..TabDecoration::default()
            }
        };
    }
    if !selected {
        return TabDecoration {
            hover_fill: Some(hover_fill),
            ..TabDecoration::default()
        };
    }
    let (fill, border) = match (active, light) {
        (true, false) => (rgb(0x233558), rgb(0x2E4D89)),
        (true, true) => (rgb(0xE3EBFE), rgb(0xA7C5FF)),
        (false, false) => (rgb(0x26282C), rgb(0x40434A)),
        (false, true) => (rgb(0xE9EAEE), rgb(0xD1D3D9)),
    };
    TabDecoration {
        fill: Some(fill.into()),
        border: Some(border.into()),
        hover_fill: (!active).then_some(hover_fill),
        underline: None,
    }
}

pub(crate) fn pill_vertical_geometry(tab_height: Pixels) -> (Pixels, Pixels) {
    let height = f32::from(tab_height);
    let vertical_offset = (height - PILL_FULL_HEIGHT).max(PILL_MIN_VERTICAL_OFFSET);
    (
        px((vertical_offset / 2.).floor()),
        px((height - vertical_offset).max(0.)),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabMenuCommand {
    CloseTab,
    CloseAllTabs,
    CloseOtherTabs,
    SelectNextTab,
    SelectPreviousTab,
    ShowListOfTabs,
}

impl TabMenuCommand {
    pub(crate) fn label(self) -> &'static str {
        match self {
            TabMenuCommand::CloseTab => "Close Tab",
            TabMenuCommand::CloseAllTabs => "Close All Tabs",
            TabMenuCommand::CloseOtherTabs => "Close Other Tabs",
            TabMenuCommand::SelectNextTab => "Select Next Tab",
            TabMenuCommand::SelectPreviousTab => "Select Previous Tab",
            TabMenuCommand::ShowListOfTabs => "Show List of Tabs",
        }
    }

    /// The action whose key binding the entry shows, like IntelliJ's content menu, which copies the
    /// shortcuts of CloseActiveTab, NextTab, PreviousTab and ShowContent.
    pub(crate) fn shortcut_action(self) -> Option<Box<dyn Action>> {
        match self {
            TabMenuCommand::CloseTab => Some(Box::new(CloseActiveTab)),
            TabMenuCommand::SelectNextTab => Some(Box::new(SelectNextTab)),
            TabMenuCommand::SelectPreviousTab => Some(Box::new(SelectPreviousTab)),
            TabMenuCommand::ShowListOfTabs => Some(Box::new(ShowListOfTabs)),
            TabMenuCommand::CloseAllTabs | TabMenuCommand::CloseOtherTabs => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabMenuItem {
    Separator,
    Command {
        command: TabMenuCommand,
        enabled: bool,
    },
}

pub(crate) fn tab_menu_items(state: &TabMenuState) -> Vec<TabMenuItem> {
    let command =
        |command: TabMenuCommand, enabled: bool| TabMenuItem::Command { command, enabled };
    let mut items = Vec::new();
    if state.closeable {
        items.push(command(TabMenuCommand::CloseTab, true));
    }
    items.push(command(TabMenuCommand::CloseAllTabs, state.can_close_all));
    items.push(command(
        TabMenuCommand::CloseOtherTabs,
        state.can_close_others,
    ));
    items.push(TabMenuItem::Separator);
    items.push(command(TabMenuCommand::SelectNextTab, state.can_navigate));
    items.push(command(
        TabMenuCommand::SelectPreviousTab,
        state.can_navigate,
    ));
    // IntelliJ's ShowContent is only visible while the tool window has more than one tab.
    if state.can_navigate {
        items.push(command(TabMenuCommand::ShowListOfTabs, true));
    }
    items
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabListEntry {
    pub key: ContentTabKey,
    pub title: SharedString,
    pub selected: bool,
}

pub(crate) fn tab_width(text_width: Pixels, closeable: bool) -> Pixels {
    let width = if closeable {
        TAB_HORIZONTAL_INSET * 2. + text_width + CLOSE_ICON_SIZE + CLOSE_ICONS_GAP
    } else {
        TAB_HORIZONTAL_INSET * 2. + text_width
    };
    width.min(MAX_TAB_WIDTH)
}

pub(crate) struct GitToolWindowHeader {
    tool_window: WeakEntity<GitToolWindow>,
    measured_width: Rc<Cell<Option<Pixels>>>,
    tab_list_anchor: Rc<Cell<Option<Point<Pixels>>>>,
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
}

impl GitToolWindowHeader {
    pub(crate) fn new(tool_window: WeakEntity<GitToolWindow>) -> Self {
        Self {
            tool_window,
            measured_width: Rc::new(Cell::new(None)),
            tab_list_anchor: Rc::new(Cell::new(None)),
            context_menu: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn has_open_menu(&self) -> bool {
        self.context_menu.is_some()
    }

    fn show_menu(
        &mut self,
        menu: Entity<ContextMenu>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&menu.focus_handle(cx), cx);
        let subscription = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.context_menu = None;
            cx.notify();
        });
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }

    pub(crate) fn deploy_tab_list(
        &mut self,
        tabs: Vec<TabListEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.tab_list_anchor.get().unwrap_or_default();
        let weak_tool_window = self.tool_window.clone();
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            tabs.into_iter().fold(menu, |menu, tab| {
                let tool_window = weak_tool_window.clone();
                let key = tab.key;
                menu.toggleable_entry(
                    tab.title,
                    tab.selected,
                    IconPosition::Start,
                    None,
                    move |window, cx| {
                        tool_window
                            .update(cx, |tool_window, cx| {
                                tool_window.select_tab(&key, true, window, cx)
                            })
                            .log_err();
                    },
                )
            })
        });
        menu.update(cx, |menu, cx| menu.select_toggled_or_first(window, cx));
        self.show_menu(menu, position, window, cx);
    }

    fn update_tool_window(
        &self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut GitToolWindow, &mut Context<GitToolWindow>),
    ) {
        self.tool_window.update(cx, update).log_err();
    }

    fn tab_left_click(
        &mut self,
        key: ContentTabKey,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if event.modifiers.alt {
            self.update_tool_window(cx, |tool_window, cx| {
                tool_window.close_other_tabs(&key, window, cx)
            });
            return;
        }
        let close_click = event.modifiers.shift;
        self.update_tool_window(cx, |tool_window, cx| {
            tool_window.select_tab(&key, true, window, cx);
            if close_click {
                tool_window.process_close_click(&key, window, cx);
            }
        });
    }

    fn deploy_tab_menu(
        &mut self,
        key: ContentTabKey,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tool_window) = self.tool_window.upgrade() else {
            return;
        };
        let tool_window = tool_window.read(cx);
        let items = tab_menu_items(&tool_window.tab_menu_state(&key));
        let shortcut_context = tool_window.shortcut_focus_handle();
        let weak_tool_window = self.tool_window.clone();
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            items
                .into_iter()
                .fold(menu.context(shortcut_context), |menu, item| match item {
                    TabMenuItem::Separator => menu.separator(),
                    TabMenuItem::Command { command, enabled } => {
                        let tool_window = weak_tool_window.clone();
                        let key = key.clone();
                        let entry = ContextMenuEntry::new(command.label())
                            .disabled(!enabled)
                            .handler(move |window, cx| {
                                tool_window
                                    .update(cx, |tool_window, cx| {
                                        tool_window.run_tab_menu_command(command, &key, window, cx)
                                    })
                                    .log_err();
                            });
                        match command.shortcut_action() {
                            Some(action) => menu.item(entry.action(action)),
                            None => menu.item(entry),
                        }
                    }
                })
        });
        self.show_menu(menu, position, window, cx);
    }

    fn render_tab(
        &self,
        index: usize,
        tab: &HeaderTab,
        width: Pixels,
        snapshot: &HeaderSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let islands = snapshot.islands;
        let decoration = tab_decoration(tab.selected, snapshot.active, snapshot.light, islands);
        let (pill_top, pill_height) = pill_vertical_geometry(snapshot.header_height);
        let group_name = SharedString::from(format!("git-tool-window-tab-{index}"));
        let close_group_name = SharedString::from(format!("git-tool-window-tab-close-{index}"));
        let key = tab.key.clone();

        let mut pill = div()
            .absolute()
            .left(PILL_HORIZONTAL_OFFSET)
            .right(PILL_HORIZONTAL_OFFSET);
        pill = if islands {
            pill.top(pill_top).h(pill_height).rounded(PILL_RADIUS)
        } else {
            pill.top_0().bottom_0()
        };
        if let Some(fill) = decoration.fill {
            pill = pill.bg(fill);
        }
        if let Some(border) = decoration.border {
            pill = pill.border_1().border_color(border);
        }
        if let Some(hover_fill) = decoration.hover_fill {
            pill = pill.group_hover(group_name.clone(), move |style| style.bg(hover_fill));
        }

        let close_button = tab.closeable.then(|| {
            let close_key = key.clone();
            let tooltip_handle = snapshot.focus_handle.clone();
            let selected = tab.selected;
            div()
                .id(("git-tool-window-tab-close", index))
                .group(close_group_name.clone())
                .flex_none()
                .relative()
                .size(CLOSE_ICON_SIZE)
                .ml(CLOSE_ICON_TEXT_GAP)
                .mt(CLOSE_ICON_BOTTOM_SHIFT)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .group_hover(close_group_name.clone(), |style| style.invisible())
                        .child(
                            Icon::new(IconName::CloseSmall)
                                .size(IconSize::Medium)
                                .color(Color::Muted),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .invisible()
                        .group_hover(close_group_name.clone(), |style| style.visible())
                        .child(
                            Icon::new(IconName::CloseSmallHovered)
                                .size(IconSize::Medium)
                                .color(Color::Muted),
                        ),
                )
                .tooltip(move |window, cx| {
                    if selected {
                        Tooltip::for_action_in(
                            CLOSE_TAB_TOOLTIP,
                            &CloseActiveTab,
                            &tooltip_handle,
                            cx,
                        )
                    } else {
                        Tooltip::text(CLOSE_TAB_TOOLTIP)(window, cx)
                    }
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, window, cx| {
                        cx.stop_propagation();
                        this.update_tool_window(cx, |tool_window, cx| {
                            tool_window.close_tab(&close_key, window, cx)
                        });
                    }),
                )
        });

        let left_key = key.clone();
        let middle_key = key.clone();
        let menu_key = key;
        div()
            .id(("git-tool-window-tab", index))
            .group(group_name)
            .relative()
            .flex_none()
            .h_full()
            .w(width)
            .overflow_hidden()
            .child(pill)
            .when_some(decoration.underline, |this, underline| {
                this.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(UNDERLINE_HEIGHT)
                        .rounded(UNDERLINE_RADIUS)
                        .bg(underline),
                )
            })
            .child(
                h_flex()
                    .relative()
                    .h_full()
                    .w_full()
                    .pl(TAB_HORIZONTAL_INSET)
                    .pr(if tab.closeable {
                        CLOSE_ICON_RIGHT_INSET
                    } else {
                        TAB_HORIZONTAL_INSET
                    })
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Label::new(tab.title.clone()).truncate()),
                    )
                    .children(close_button),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseUpEvent, window, cx| {
                    this.tab_left_click(left_key.clone(), event, window, cx)
                }),
            )
            .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, _: &MouseUpEvent, window, cx| {
                    cx.stop_propagation();
                    this.update_tool_window(cx, |tool_window, cx| {
                        tool_window.process_close_click(&middle_key, window, cx)
                    });
                }),
            )
            .capture_any_mouse_down(
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    if event.button == MouseButton::Right {
                        cx.stop_propagation();
                        window.prevent_default();
                        this.deploy_tab_menu(menu_key.clone(), event.position, window, cx);
                    }
                }),
            )
            .into_any_element()
    }

    fn render_dropdown(&self, snapshot: &HeaderSnapshot) -> AnyElement {
        let tool_window = self.tool_window.clone();
        let focus_handle = snapshot.focus_handle.clone();
        h_flex()
            .flex_none()
            .size(TOOLBAR_BUTTON_SIZE)
            .justify_center()
            .child(
                PopoverMenu::new("git-tool-window-tab-actions")
                    .anchor(Anchor::TopLeft)
                    .menu(move |window, cx| {
                        let state = tool_window.upgrade()?.read(cx).dropdown_state(cx);
                        let console_tool_window = tool_window.clone();
                        let focus_handle = focus_handle.clone();
                        Some(ContextMenu::build(window, cx, move |menu, _, _| {
                            menu.context(focus_handle)
                                .when_some(state.show_history, |menu, (label, enabled)| {
                                    menu.action_disabled_when(
                                        !enabled,
                                        label,
                                        Box::new(git::FileHistory),
                                    )
                                })
                                .when(state.show_console_tab, move |menu| {
                                    menu.entry("Show VCS Console Tab", None, move |window, cx| {
                                        console_tool_window
                                            .update(cx, |tool_window, cx| {
                                                tool_window.show_console_tab(window, cx)
                                            })
                                            .log_err();
                                    })
                                })
                                .when(state.show_worktrees, |menu| {
                                    menu.action(
                                        "Worktrees\u{2026}",
                                        Box::new(zed_actions::git::Worktree),
                                    )
                                })
                        }))
                    })
                    .trigger(
                        IconButton::new(
                            "git-tool-window-tab-actions-button",
                            IconName::ChevronDownLarge,
                        )
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Medium),
                    ),
            )
            .into_any_element()
    }

    fn render_hidden_tabs_button(&self, hidden: Vec<(ContentTabKey, SharedString)>) -> AnyElement {
        let tool_window = self.tool_window.clone();
        div()
            .ml_auto()
            .flex_none()
            .h_full()
            .flex()
            .items_center()
            .child(
                h_flex().size(TOOLBAR_BUTTON_SIZE).justify_center().child(
                    PopoverMenu::new("git-tool-window-hidden-tabs")
                        .anchor(Anchor::TopRight)
                        .menu(move |window, cx| {
                            let hidden = hidden.clone();
                            let tool_window = tool_window.clone();
                            Some(ContextMenu::build(window, cx, move |mut menu, _, _| {
                                for (key, title) in hidden {
                                    let tool_window = tool_window.clone();
                                    menu = menu.entry(title, None, move |window, cx| {
                                        tool_window
                                            .update(cx, |tool_window, cx| {
                                                tool_window.select_tab(&key, true, window, cx)
                                            })
                                            .log_err();
                                    });
                                }
                                menu
                            }))
                        })
                        .trigger_with_tooltip(
                            IconButton::new(
                                "git-tool-window-hidden-tabs-button",
                                IconName::ChevronDownLarge,
                            )
                            .shape(IconButtonShape::Square)
                            .icon_size(IconSize::Medium),
                            move |_, cx| {
                                Tooltip::with_meta(
                                    SHOW_HIDDEN_TABS_TEXT,
                                    None,
                                    SHOW_HIDDEN_TABS_DESCRIPTION,
                                    cx,
                                )
                            },
                        ),
                ),
            )
            .into_any_element()
    }
}

impl Render for GitToolWindowHeader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(tool_window) = self.tool_window.upgrade() else {
            return div().into_any_element();
        };
        let snapshot = tool_window.read(cx).header_snapshot(window, cx);

        let font_size = TextSize::Default.pixels(cx);
        let text_style = window.text_style();
        let widths: Vec<Pixels> = snapshot
            .tabs
            .iter()
            .map(|tab| {
                let run = text_style.to_run(tab.title.len());
                let text_width = window
                    .text_system()
                    .shape_line(tab.title.clone(), font_size, &[run], None)
                    .width;
                tab_width(text_width, tab.closeable)
            })
            .collect();
        let selected_index = snapshot.tabs.iter().position(|tab| tab.selected);
        let toolbar_buttons =
            usize::from(snapshot.can_open_log_tab) + usize::from(snapshot.dropdown_visible);
        let toolbar_width = TOOLBAR_BUTTON_SIZE * toolbar_buttons as f32;
        let measured_width = self.measured_width.get();
        let mut layout: TabStripLayout =
            layout_tab_strip(&widths, selected_index, measured_width, toolbar_width);
        if layout.has_dropped_tabs {
            layout = layout_tab_strip(
                &widths,
                selected_index,
                measured_width.map(|width| width - TOOLBAR_BUTTON_SIZE),
                toolbar_width,
            );
        }
        let hidden_tabs: Vec<(ContentTabKey, SharedString)> = layout
            .hidden_indices()
            .into_iter()
            .filter_map(|index| snapshot.tabs.get(index))
            .map(|tab| (tab.key.clone(), tab.title.clone()))
            .collect();

        let tab_elements: Vec<AnyElement> = snapshot
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                let slot = layout.slots.get(index).copied().unwrap_or(TabSlot::Full);
                if slot == TabSlot::Hidden {
                    return None;
                }
                let width = match slot {
                    TabSlot::Squeezed(width) => width,
                    TabSlot::Full | TabSlot::Hidden => widths.get(index).copied().unwrap_or(px(0.)),
                };
                Some(self.render_tab(index, tab, width, &snapshot, cx))
            })
            .collect();

        let measured = self.measured_width.clone();
        let tab_list_anchor = self.tab_list_anchor.clone();
        let header = cx.weak_entity();
        let open_log_tab = snapshot.can_open_log_tab.then(|| {
            h_flex()
                .flex_none()
                .size(TOOLBAR_BUTTON_SIZE)
                .justify_center()
                .child(
                    IconButton::new("git-tool-window-open-log-tab", IconName::GeneralAdd)
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Medium)
                        .tooltip(move |_, cx| {
                            Tooltip::with_meta(
                                OPEN_LOG_TAB_TEXT,
                                None,
                                OPEN_LOG_TAB_DESCRIPTION,
                                cx,
                            )
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.update_tool_window(cx, |tool_window, cx| {
                                tool_window.open_another_log_tab(window, cx)
                            });
                        })),
                )
        });
        let dropdown = snapshot
            .dropdown_visible
            .then(|| self.render_dropdown(&snapshot));
        let hidden_tabs_button = (layout.has_dropped_tabs && !hidden_tabs.is_empty())
            .then(|| self.render_hidden_tabs_button(hidden_tabs));

        h_flex()
            .id("git-tool-window-header-tabs")
            .relative()
            .size_full()
            .items_center()
            .child(
                canvas(
                    move |bounds, window, _| {
                        tab_list_anchor.set(Some(bounds.bottom_left()));
                        let width = bounds.size.width;
                        let changed = measured.get().is_none_or(|previous| {
                            (f32::from(previous) - f32::from(width)).abs() > WIDTH_CHANGE_TOLERANCE
                        });
                        measured.set(Some(width));
                        if changed {
                            window.on_next_frame(move |_, cx| {
                                header.update(cx, |_, cx| cx.notify()).log_err();
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .children(tab_elements)
            .children(open_log_tab)
            .children(dropdown)
            .children(hidden_tabs_button)
            .children(self.context_menu.as_ref().map(|(menu, position, _)| {
                deferred(anchored().position(*position).child(menu.clone())).with_priority(1)
            }))
            .into_any_element()
    }
}
