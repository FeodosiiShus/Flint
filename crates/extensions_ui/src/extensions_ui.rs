mod components;
mod extension_suggestions;

use std::{any::TypeId, ops::Range, sync::Arc};

use cloud_api_types::ExtensionProvides;
use command_palette_hooks::CommandPaletteFilter;
use editor::{Editor, EditorElement, EditorStyle};
use extension_host::{ExtensionIndexEntry, ExtensionManifest, ExtensionStore};
use fuzzy::{StringMatch, StringMatchCandidate, match_strings};
use git::{GitHostingProviderRegistry, parse_git_remote_url};
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, Focusable, InteractiveElement, KeyContext,
    ParentElement, Render, Styled, Task, TextStyle, UniformListScrollHandle, Window, actions,
    point, uniform_list,
};

use picker::{Picker, PickerDelegate};
use project::DirectoryLister;

use schemars::JsonSchema;
use serde::Deserialize;
use settings::Settings;
use strum::IntoEnumIterator as _;
use theme_settings::ThemeSettings;
use ui::{
    ListItem, ListItemSpacing, ScrollableHandle, ToggleButtonGroup, ToggleButtonGroupSize,
    ToggleButtonGroupStyle, ToggleButtonSimple, WithScrollbar, prelude::*,
};
use util::ResultExt;
use workspace::{
    Workspace,
    item::{Item, ItemEvent},
    workspace_error::{ErrorAction, ErrorSeverity, WorkspaceError},
};
use zed_actions::ExtensionCategoryFilter;

use crate::components::{ExtensionCard, extension_provides_label};

actions!(
    zed,
    [
        /// Installs an extension from a local directory for development.
        InstallDevExtension,
    ]
);

/// Rebuilds an installed dev extension.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema, gpui::Action)]
#[action(namespace = zed)]
#[serde(deny_unknown_fields)]
pub struct RebuildDevExtension {
    /// The ID of the dev extension to rebuild.
    ///
    /// Default: opens a picker if multiple dev extensions are installed.
    #[serde(default)]
    pub extension_id: Option<String>,
}

#[derive(Default)]
struct DevExtensionNotInstalledError {
    extension_id: Option<SharedString>,
}

impl WorkspaceError for DevExtensionNotInstalledError {
    fn primary_message(&self) -> SharedString {
        match &self.extension_id {
            Some(extension_id) => {
                format!("Dev extension '{extension_id}' is not installed.").into()
            }
            None => "No dev extensions are installed.".into(),
        }
    }

    fn primary_action(&self) -> ErrorAction {
        ErrorAction::new("Install Dev Extension", InstallDevExtension)
    }

    fn severity(&self) -> ErrorSeverity {
        ErrorSeverity::Warning
    }
}

fn update_rebuild_dev_extension_visibility(store: &Entity<ExtensionStore>, cx: &mut App) {
    let has_dev_extensions = store.read(cx).dev_extensions().next().is_some();
    CommandPaletteFilter::update_global(cx, |filter, _cx| {
        if has_dev_extensions {
            filter.show_action_types(&[TypeId::of::<RebuildDevExtension>()]);
        } else {
            filter.hide_action_types(&[TypeId::of::<RebuildDevExtension>()]);
        }
    });
}

pub fn init(cx: &mut App) {
    let store = ExtensionStore::global(cx);
    update_rebuild_dev_extension_visibility(&store, cx);
    cx.observe(&store, |store, cx| {
        update_rebuild_dev_extension_visibility(&store, cx);
    })
    .detach();
    extension_suggestions::init(cx);

    cx.observe_new(move |workspace: &mut Workspace, window, _cx| {
        if window.is_none() {
            return;
        }
        workspace
            .register_action(
                move |workspace, action: &zed_actions::Extensions, window, cx| {
                    let provides_filter = action.category_filter.map(|category| match category {
                        ExtensionCategoryFilter::Themes => ExtensionProvides::Themes,
                        ExtensionCategoryFilter::IconThemes => ExtensionProvides::IconThemes,
                        ExtensionCategoryFilter::Languages => ExtensionProvides::Languages,
                        ExtensionCategoryFilter::Grammars => ExtensionProvides::Grammars,
                        ExtensionCategoryFilter::LanguageServers => {
                            ExtensionProvides::LanguageServers
                        }
                        ExtensionCategoryFilter::Snippets => ExtensionProvides::Snippets,
                        ExtensionCategoryFilter::DebugAdapters => ExtensionProvides::DebugAdapters,
                    });

                    let existing = workspace
                        .active_pane()
                        .read(cx)
                        .items()
                        .find_map(|item| item.downcast::<ExtensionsPage>());

                    if let Some(existing) = existing {
                        existing.update(cx, |extensions_page, cx| {
                            if provides_filter.is_some() {
                                extensions_page.change_provides_filter(provides_filter, cx);
                            }
                            if let Some(id) = action.id.as_ref() {
                                extensions_page.focus_extension(id, window, cx);
                            }
                        });

                        workspace.activate_item(&existing, true, true, window, cx);
                    } else {
                        let extensions_page =
                            ExtensionsPage::new(provides_filter, action.id.as_deref(), window, cx);
                        workspace.add_item_to_active_pane(
                            Box::new(extensions_page),
                            None,
                            true,
                            window,
                            cx,
                        )
                    }
                },
            )
            .register_action(move |workspace, _: &InstallDevExtension, window, cx| {
                let store = ExtensionStore::global(cx);
                let prompt = workspace.prompt_for_open_path(
                    gpui::PathPromptOptions {
                        files: false,
                        directories: true,
                        multiple: false,
                        prompt: None,
                    },
                    DirectoryLister::Local(
                        workspace.project().clone(),
                        workspace.app_state().fs.clone(),
                    ),
                    window,
                    cx,
                );

                let workspace_handle = cx.entity().downgrade();
                window
                    .spawn(cx, async move |cx| {
                        let extension_path = match prompt.await.map_err(anyhow::Error::from) {
                            Ok(Some(mut paths)) => paths.pop()?,
                            Ok(None) => return None,
                            Err(err) => {
                                workspace_handle
                                    .update(cx, |workspace, cx| {
                                        workspace.show_error(
                                            workspace::workspace_error::PortalError::new(
                                                err.to_string(),
                                            ),
                                            cx,
                                        );
                                    })
                                    .ok();
                                return None;
                            }
                        };

                        let install_task = store.update(cx, |store, cx| {
                            store.install_dev_extension(extension_path, cx)
                        });

                        match install_task.await {
                            Ok(_) => {}
                            Err(err) => {
                                log::error!("Failed to install dev extension: {:?}", err);
                                workspace_handle
                                    .update(cx, |workspace, cx| {
                                        // NOTE: using `anyhow::context` here ends up not printing
                                        // the error
                                        workspace.show_error(
                                            format!("Failed to install dev extension: {}", err),
                                            cx,
                                        );
                                    })
                                    .ok();
                            }
                        }

                        Some(())
                    })
                    .detach();
            })
            .register_action(move |workspace, action: &RebuildDevExtension, window, cx| {
                if let Some(target_id) = action.extension_id.as_deref() {
                    let extension_id = ExtensionStore::global(cx)
                        .read(cx)
                        .dev_extensions()
                        .find_map(|m| {
                            if m.id.as_ref() == target_id {
                                Some(m.id.clone())
                            } else {
                                None
                            }
                        });
                    if let Some(extension_id) = extension_id {
                        ExtensionStore::global(cx).update(cx, |store, cx| {
                            store.rebuild_dev_extension(extension_id, cx);
                        });
                    } else {
                        workspace.show_error(
                            DevExtensionNotInstalledError {
                                extension_id: Some(SharedString::from(target_id.to_owned())),
                            },
                            cx,
                        );
                    }
                    return;
                }

                let dev_extensions = ExtensionStore::global(cx)
                    .read(cx)
                    .dev_extensions()
                    .cloned()
                    .collect::<Vec<_>>();

                match dev_extensions.len() {
                    0 => {
                        workspace.show_error(DevExtensionNotInstalledError::default(), cx);
                    }
                    1 => {
                        let extension_id = dev_extensions[0].id.clone();
                        ExtensionStore::global(cx).update(cx, |store, cx| {
                            store.rebuild_dev_extension(extension_id, cx);
                        });
                    }
                    _ => {
                        workspace.toggle_modal(window, cx, |window, cx| {
                            let delegate = DevExtensionRebuildPickerDelegate::new(dev_extensions);
                            Picker::uniform_list(delegate, window, cx)
                        });
                    }
                }
            });
    })
    .detach();
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
enum ExtensionFilter {
    All,
    Installed,
}

#[derive(Clone, Copy)]
enum DisplayedExtension {
    Installed(usize),
}

pub struct ExtensionsPage {
    extension_store: Entity<ExtensionStore>,
    provider_registry: Arc<GitHostingProviderRegistry>,
    list: UniformListScrollHandle,
    filter: ExtensionFilter,
    installed_search_results: Vec<ExtensionIndexEntry>,
    displayed_extensions: Vec<DisplayedExtension>,
    query_editor: Entity<Editor>,
    query_contains_error: bool,
    provides_filter: Option<ExtensionProvides>,
    _subscriptions: [gpui::Subscription; 2],
    local_search_task: Option<Task<()>>,
}

impl ExtensionsPage {
    pub fn new(
        provides_filter: Option<ExtensionProvides>,
        focus_extension_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let extension_store = ExtensionStore::global(cx);
            let subscriptions = [
                cx.observe(&extension_store, |_: &mut Self, _, cx| cx.notify()),
                cx.subscribe(&extension_store, |this, _, event, cx| {
                    if let extension_host::Event::ExtensionsUpdated = event {
                        this.update_local_search_results(cx)
                    }
                }),
            ];

            let query_editor = cx.new(|cx| {
                let mut input = Editor::single_line(window, cx);
                input.set_placeholder_text("Search extensions...", window, cx);
                if let Some(id) = focus_extension_id {
                    input.set_text(format!("id:{id}"), window, cx);
                }
                input
            });
            cx.subscribe(&query_editor, Self::on_query_change).detach();

            let scroll_handle = UniformListScrollHandle::new();
            let provider_registry = GitHostingProviderRegistry::default_global(cx);

            let mut this = Self {
                extension_store,
                provider_registry,
                list: scroll_handle,
                filter: ExtensionFilter::All,
                installed_search_results: Vec::new(),
                displayed_extensions: Vec::new(),
                query_contains_error: false,
                provides_filter,
                local_search_task: None,
                _subscriptions: subscriptions,
                query_editor,
            };
            this.update_local_search_results(cx);
            this
        })
    }

    fn get_repository_icon(&self, repository_url: &str) -> IconName {
        parse_git_remote_url(Arc::clone(&self.provider_registry), repository_url)
            .map(|(provider, _)| ui::git_hosting_provider_icon(provider.name().as_str()))
            .unwrap_or(IconName::Link)
    }

    /// Runs the search against the locally installed extensions.
    fn update_local_search_results(&mut self, cx: &mut Context<Self>) {
        let search = self.search_query(cx);
        let mut installed_extensions = self
            .extension_store
            .read(cx)
            .installed_extensions()
            .values()
            .cloned()
            .collect::<Vec<_>>();

        self.local_search_task = Some(cx.spawn(async move |this, cx| {
            let results = match search.as_deref() {
                None => {
                    installed_extensions
                        .sort_by_cached_key(|extension| extension.manifest.name.to_lowercase());
                    installed_extensions
                }
                Some(search) => {
                    if let Some(extension_id) = search.strip_prefix("id:") {
                        installed_extensions
                            .into_iter()
                            .filter(|extension| extension.manifest.id.as_ref() == extension_id)
                            .collect()
                    } else {
                        let match_candidates = installed_extensions
                            .iter()
                            .enumerate()
                            .map(|(index, extension)| {
                                StringMatchCandidate::new(index, &extension.manifest.name)
                            })
                            .collect::<Vec<_>>();
                        let matches = match_strings(
                            &match_candidates,
                            search,
                            false,
                            true,
                            match_candidates.len(),
                            &Default::default(),
                            cx.background_executor().clone(),
                        )
                        .await;
                        matches
                            .into_iter()
                            .filter_map(|matched| {
                                installed_extensions.get(matched.candidate_id).cloned()
                            })
                            .collect()
                    }
                }
            };

            this.update(cx, |this, cx| {
                this.installed_search_results = results;
                this.rebuild_displayed_extensions(cx);
            })
            .ok();
        }));
    }

    fn rebuild_displayed_extensions(&mut self, cx: &mut Context<Self>) {
        let provides_filter = self.provides_filter;
        let installed_only = self.filter == ExtensionFilter::Installed;
        self.displayed_extensions = self
            .installed_search_results
            .iter()
            .enumerate()
            .filter(|(_, extension)| {
                (!installed_only || !extension.dev)
                    && provides_filter
                        .is_none_or(|provides| extension.manifest.provides().contains(&provides))
            })
            .map(|(index, _)| DisplayedExtension::Installed(index))
            .collect();
        cx.notify();
    }

    fn scroll_to_top(&mut self, cx: &mut Context<Self>) {
        self.list.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn render_extensions(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<ExtensionCard> {
        let extension_store = self.extension_store.read(cx);
        range
            .filter_map(|index| {
                let row = *self.displayed_extensions.get(index)?;
                self.render_extension(row, extension_store)
            })
            .collect()
    }

    fn render_extension(
        &self,
        row: DisplayedExtension,
        extension_store: &ExtensionStore,
    ) -> Option<ExtensionCard> {
        let DisplayedExtension::Installed(index) = row;
        let extension = self.installed_search_results.get(index)?;
        let manifest = extension.manifest.clone();
        let repository_icon = manifest
            .repository
            .as_deref()
            .map(|url| self.get_repository_icon(url));
        let card = if extension.dev {
            ExtensionCard::for_dev(manifest, extension_store)
        } else {
            ExtensionCard::for_installed(manifest, extension_store)
        };
        Some(match repository_icon {
            Some(icon) => card.repository_icon(icon),
            None => card,
        })
    }

    fn render_search(&self, cx: &mut Context<Self>) -> Div {
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("BufferSearchBar");

        let editor_border = if self.query_contains_error {
            Color::Error.color(cx)
        } else {
            cx.theme().colors().border
        };

        h_flex()
            .key_context(key_context)
            .h_8()
            .min_w(rems_from_px(384_f32))
            .flex_1()
            .pl_1p5()
            .pr_2()
            .gap_2()
            .border_1()
            .border_color(editor_border)
            .rounded_md()
            .child(Icon::new(IconName::MagnifyingGlass).color(Color::Muted))
            .child(
                div()
                    .flex_1()
                    .child(self.render_text_input(&self.query_editor, cx)),
            )
    }

    fn render_text_input(
        &self,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let settings = ThemeSettings::get_global(cx);
        let text_style = TextStyle {
            color: if editor.read(cx).read_only(cx) {
                cx.theme().colors().text_disabled
            } else {
                cx.theme().colors().text
            },
            font_family: settings.ui_font.family.clone(),
            font_features: settings.ui_font.features.clone(),
            font_fallbacks: settings.ui_font.fallbacks.clone(),
            font_size: rems(0.875).into(),
            font_weight: settings.ui_font.weight,
            line_height: relative(1.3),
            ..Default::default()
        };

        EditorElement::new(
            editor,
            EditorStyle {
                background: cx.theme().colors().editor_background,
                local_player: cx.theme().players().local(),
                text: text_style,
                ..Default::default()
            },
        )
    }

    fn on_query_change(
        &mut self,
        _: Entity<Editor>,
        event: &editor::EditorEvent,
        cx: &mut Context<Self>,
    ) {
        if let editor::EditorEvent::Edited { .. } = event {
            self.query_contains_error = false;
            self.refresh_search(cx);
        }
    }

    fn refresh_search(&mut self, cx: &mut Context<Self>) {
        self.update_local_search_results(cx);
        self.scroll_to_top(cx);
    }

    pub fn focus_extension(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query_editor.update(cx, |editor, cx| {
            editor.set_text(format!("id:{id}"), window, cx)
        });
        self.refresh_search(cx);
    }

    pub fn change_provides_filter(
        &mut self,
        provides_filter: Option<ExtensionProvides>,
        cx: &mut Context<Self>,
    ) {
        self.provides_filter = provides_filter;
        self.refresh_search(cx);
    }

    pub fn search_query(&self, cx: &mut App) -> Option<String> {
        let search = self.query_editor.read(cx).text(cx);
        if search.trim().is_empty() {
            None
        } else {
            Some(search)
        }
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_search = self.search_query(cx).is_some();
        let message = match self.filter {
            ExtensionFilter::All => {
                if has_search {
                    "No extensions that match your search."
                } else {
                    "No extensions."
                }
            }
            ExtensionFilter::Installed => {
                if has_search {
                    "No installed extensions that match your search."
                } else {
                    "No installed extensions."
                }
            }
        };

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(h_flex().gap_1p5().child(Label::new(message)))
    }
}

struct DevExtensionRebuildPickerDelegate {
    entries: Vec<Arc<ExtensionManifest>>,
    matches: Vec<StringMatch>,
    selected_index: usize,
}

impl DevExtensionRebuildPickerDelegate {
    fn new(manifests: Vec<Arc<ExtensionManifest>>) -> Self {
        let matches = manifests
            .iter()
            .enumerate()
            .map(|(ix, manifest)| StringMatch {
                candidate_id: ix,
                score: 0.0,
                positions: Vec::new(),
                string: manifest.name.clone(),
            })
            .collect();

        Self {
            entries: manifests,
            matches,
            selected_index: 0,
        }
    }
}

impl PickerDelegate for DevExtensionRebuildPickerDelegate {
    type ListItem = ListItem;

    fn name() -> &'static str {
        "dev-extension-rebuild"
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(
        &mut self,
        ix: usize,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn selected_index_changed(
        &self,
        _ix: usize,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Option<Box<dyn Fn(&mut Window, &mut App) + 'static>> {
        None
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        let background = cx.background_executor().clone();
        let candidates = self
            .entries
            .iter()
            .enumerate()
            .map(|(ix, manifest)| StringMatchCandidate::new(ix, manifest.name.as_ref()))
            .collect::<Vec<_>>();

        cx.spawn_in(window, async move |this, cx| {
            let matches = if query.is_empty() {
                candidates
                    .into_iter()
                    .enumerate()
                    .map(|(index, candidate)| StringMatch {
                        candidate_id: index,
                        string: candidate.string,
                        positions: Vec::new(),
                        score: 0.0,
                    })
                    .collect()
            } else {
                match_strings(
                    &candidates,
                    &query,
                    false,
                    true,
                    100,
                    &Default::default(),
                    background,
                )
                .await
            };

            this.update(cx, |this, _cx| {
                this.delegate.matches = matches;
                this.delegate.selected_index = this
                    .delegate
                    .selected_index
                    .min(this.delegate.matches.len().saturating_sub(1));
            })
            .log_err();
        })
    }

    fn confirm(&mut self, _secondary: bool, _window: &mut Window, cx: &mut Context<Picker<Self>>) {
        let Some(mat) = self.matches.get(self.selected_index) else {
            return;
        };

        let extension_id = self.entries[mat.candidate_id].id.clone();
        ExtensionStore::global(cx).update(cx, |store, cx| {
            store.rebuild_dev_extension(extension_id, cx);
        });

        cx.emit(DismissEvent);
    }

    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {}

    fn placeholder_text(&self, _window: &mut Window, _cx: &mut App) -> Arc<str> {
        Arc::from("Rebuild dev extension…")
    }

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let mat = self.matches.get(ix)?;
        let entry = self.entries.get(mat.candidate_id)?;

        let item = ListItem::new(("dev-extension-list-item", mat.candidate_id))
            .inset(true)
            .spacing(ListItemSpacing::Sparse)
            .toggle_state(selected)
            .child(
                h_flex()
                    .w_full()
                    .py_px()
                    .justify_between()
                    .gap_2()
                    .child(Label::new(entry.name.clone()))
                    .child(
                        Label::new(format!("{} • v{}", entry.id, entry.version))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            );

        Some(item)
    }

    fn no_matches_text(&self, _window: &mut Window, _cx: &mut App) -> Option<SharedString> {
        Some("No dev extensions found".into())
    }
}

impl Render for ExtensionsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .child(ui::background_image_layer(
                ui::BackgroundImageTarget::EditorAndTools,
                ui::BackgroundImageArea::Window,
                cx.theme().colors().editor_background,
                true,
                gpui::Corners::default(),
            ))
            .child(
                v_flex()
                    .gap_4()
                    .pt_4()
                    .px_4()
                    .bg(cx.theme().colors().editor_background)
                    .child(ui::background_image_layer(
                        ui::BackgroundImageTarget::EditorAndTools,
                        ui::BackgroundImageArea::Window,
                        cx.theme().colors().editor_background,
                        true,
                        gpui::Corners::default(),
                    ))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1p5()
                            .justify_between()
                            .child(Headline::new("Extensions").size(HeadlineSize::Large))
                            .child(
                                Button::new("install-dev-extension", "Install Dev Extension")
                                    .style(ButtonStyle::Outlined)
                                    .size(ButtonSize::Medium)
                                    .on_click(|_event, window, cx| {
                                        window.dispatch_action(Box::new(InstallDevExtension), cx)
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .gap_2()
                            .child(self.render_search(cx))
                            .child(
                                div().child(
                                    ToggleButtonGroup::single_row(
                                        "filter-buttons",
                                        [
                                            ToggleButtonSimple::new(
                                                "All",
                                                cx.listener(|this, _event, _, cx| {
                                                    this.filter = ExtensionFilter::All;
                                                    this.rebuild_displayed_extensions(cx);
                                                    this.scroll_to_top(cx);
                                                }),
                                            ),
                                            ToggleButtonSimple::new(
                                                "Installed",
                                                cx.listener(|this, _event, _, cx| {
                                                    this.filter = ExtensionFilter::Installed;
                                                    this.rebuild_displayed_extensions(cx);
                                                    this.scroll_to_top(cx);
                                                }),
                                            ),
                                        ],
                                    )
                                    .style(ToggleButtonGroupStyle::Outlined)
                                    .size(ToggleButtonGroupSize::Custom(rems_from_px(30_f32))) // Perfectly matches the input
                                    .label_size(LabelSize::Default)
                                    .auto_width()
                                    .selected_index(match self.filter {
                                        ExtensionFilter::All => 0,
                                        ExtensionFilter::Installed => 1,
                                    })
                                    .into_any_element(),
                                ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .id("filter-row")
                    .gap_2()
                    .py_2p5()
                    .px_4()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .overflow_x_scroll()
                    .child(
                        Button::new("filter-all-categories", "All")
                            .when(self.provides_filter.is_none(), |button| {
                                button.style(ButtonStyle::Filled)
                            })
                            .when(self.provides_filter.is_some(), |button| {
                                button.style(ButtonStyle::Subtle)
                            })
                            .toggle_state(self.provides_filter.is_none())
                            .on_click(cx.listener(|this, _event, _, cx| {
                                this.change_provides_filter(None, cx);
                            })),
                    )
                    .children(
                        ExtensionProvides::iter()
                            .filter(|provides| match provides {
                                ExtensionProvides::AgentServers
                                | ExtensionProvides::ContextServers
                                | ExtensionProvides::Grammars // grammars do not add anything of value to users currently
                                | ExtensionProvides::IndexedDocsProviders
                                | ExtensionProvides::SlashCommands => false,
                                _ => true,
                            })
                            .map(|provides| {
                                let label = extension_provides_label(provides);
                                let button_id =
                                    SharedString::from(format!("filter-category-{}", label));

                                Button::new(button_id, label)
                                    .style(if self.provides_filter == Some(provides) {
                                        ButtonStyle::Filled
                                    } else {
                                        ButtonStyle::Subtle
                                    })
                                    .toggle_state(self.provides_filter == Some(provides))
                                    .on_click({
                                        cx.listener(move |this, _event, _, cx| {
                                            this.change_provides_filter(Some(provides), cx);
                                        })
                                    })
                            }),
                    ),
            )
            .child(v_flex().px_4().size_full().overflow_y_hidden().map(|this| {
                let count = self.displayed_extensions.len();

                if count == 0 {
                    this.child(self.render_empty_state(cx)).into_any_element()
                } else {
                    let scroll_handle = &self.list;
                    this.child(
                        uniform_list("entries", count, cx.processor(Self::render_extensions))
                            .flex_grow_1()
                            .pb_4()
                            .track_scroll(scroll_handle),
                    )
                    .vertical_scrollbar_for(scroll_handle, window, cx)
                    .into_any_element()
                }
            }))
    }
}

impl EventEmitter<ItemEvent> for ExtensionsPage {}

impl Focusable for ExtensionsPage {
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.query_editor.read(cx).focus_handle(cx)
    }
}

impl Item for ExtensionsPage {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        "Extensions".into()
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(workspace::item::ItemEvent)) {
        f(*event)
    }
}
