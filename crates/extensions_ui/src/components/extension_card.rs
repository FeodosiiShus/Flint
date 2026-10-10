use std::sync::Arc;

use cloud_api_types::ExtensionProvides;
use extension::{ExtensionManifest, SchemaVersion};
use extension_host::{ExtensionOperation, ExtensionStatus, ExtensionStore};
use gpui::{ElementId, SharedString, prelude::*};
use ui::{Chip, Tooltip, prelude::*};

type ExtensionCardActions = [Option<Button>; 2];

#[derive(Clone, Copy, PartialEq, Eq)]
enum LocalExtensionKind {
    Dev,
    Installed,
}

struct ExtensionCardDetails {
    name: SharedString,
    version: Arc<str>,
    description: Option<SharedString>,
    authors: SharedString,
    repository_url: Option<SharedString>,
    repository_icon: IconName,
    provided_features: Vec<&'static str>,
    source: ExtensionCardSource,
}

#[derive(Clone, Copy)]
enum ExtensionCardSource {
    Dev,
    Installed,
}

impl ExtensionCardSource {
    fn is_dev(self) -> bool {
        matches!(self, Self::Dev)
    }
}

#[derive(IntoElement, RegisterComponent)]
pub struct ExtensionCard {
    details: ExtensionCardDetails,
    actions: ExtensionCardActions,
}

impl ExtensionCard {
    pub fn for_dev(extension: Arc<ExtensionManifest>, extension_store: &ExtensionStore) -> Self {
        let status = extension_store.extension_status(&extension.id);
        Self::manifest(extension, status, LocalExtensionKind::Dev)
    }

    pub fn for_installed(
        extension: Arc<ExtensionManifest>,
        extension_store: &ExtensionStore,
    ) -> Self {
        let status = extension_store.extension_status(&extension.id);
        Self::manifest(extension, status, LocalExtensionKind::Installed)
    }
    fn manifest(
        extension: Arc<ExtensionManifest>,
        status: ExtensionStatus,
        kind: LocalExtensionKind,
    ) -> Self {
        let actions = Self::actions_for_manifest_extension(&extension, &status, kind);
        let details = ExtensionCardDetails {
            name: extension.name.clone().into(),
            version: extension.version.clone(),
            description: extension.description.clone().map(Into::into),
            authors: extension.authors.join(", ").into(),
            repository_url: extension.repository.clone().map(Into::into),
            repository_icon: IconName::Link,
            provided_features: provided_feature_labels(extension.provides()),
            source: match kind {
                LocalExtensionKind::Dev => ExtensionCardSource::Dev,
                LocalExtensionKind::Installed => ExtensionCardSource::Installed,
            },
        };

        Self { details, actions }
    }

    fn disables_actions(status: &ExtensionStatus) -> bool {
        match status {
            ExtensionStatus::Installing
            | ExtensionStatus::Upgrading
            | ExtensionStatus::Removing => true,
            ExtensionStatus::NotInstalled | ExtensionStatus::Installed(_) => false,
        }
    }

    fn button_id(extension_id: &Arc<str>, operation: ExtensionOperation) -> ElementId {
        (SharedString::from(extension_id.clone()), operation as usize).into()
    }

    fn uninstall_button(extension_id: &Arc<str>) -> Button {
        Button::new(
            Self::button_id(extension_id, ExtensionOperation::Remove),
            "Uninstall",
        )
        .on_click({
            let extension_id = extension_id.clone();
            move |_, _, cx| {
                ExtensionStore::global(cx).update(cx, |store, cx| {
                    store
                        .uninstall_extension(extension_id.clone(), cx)
                        .detach_and_log_err(cx);
                });
            }
        })
    }

    fn actions_for_manifest_extension(
        extension: &Arc<ExtensionManifest>,
        status: &ExtensionStatus,
        kind: LocalExtensionKind,
    ) -> ExtensionCardActions {
        let is_dev = kind == LocalExtensionKind::Dev;
        let rebuild = is_dev.then(|| {
            Button::new(
                SharedString::from(format!("rebuild-{}", extension.id)),
                "Rebuild",
            )
            .color(Color::Accent)
            .disabled(Self::disables_actions(status))
            .on_click({
                let extension_id = extension.id.clone();
                move |_, _, cx| {
                    ExtensionStore::global(cx).update(cx, |store, cx| {
                        store.rebuild_dev_extension(extension_id.clone(), cx)
                    });
                }
            })
        });
        let uninstall = Self::uninstall_button(&extension.id)
            .when_else(
                is_dev,
                |button| button.color(Color::Accent),
                |button| button.style(ButtonStyle::OutlinedGhost),
            )
            .disabled(Self::disables_actions(status));

        if is_dev {
            [rebuild, Some(uninstall)]
        } else {
            [None, Some(uninstall)]
        }
    }

    pub fn repository_icon(mut self, icon: IconName) -> Self {
        self.details.repository_icon = icon;
        self
    }
}

fn provided_feature_labels(
    provides: impl IntoIterator<Item = ExtensionProvides>,
) -> Vec<&'static str> {
    provides
        .into_iter()
        .filter(|provides| !provides.is_deprecated())
        .map(extension_provides_label)
        .collect()
}

pub(crate) fn extension_provides_label(provides: ExtensionProvides) -> &'static str {
    match provides {
        ExtensionProvides::Themes => "Themes",
        ExtensionProvides::IconThemes => "Icon Themes",
        ExtensionProvides::Languages => "Languages",
        ExtensionProvides::Grammars => "Grammars",
        ExtensionProvides::LanguageServers => "Language Servers",
        ExtensionProvides::ContextServers => "MCP Servers",
        ExtensionProvides::AgentServers => "Agent Servers",
        ExtensionProvides::SlashCommands => "Slash Commands",
        ExtensionProvides::IndexedDocsProviders => "Indexed Docs Providers",
        ExtensionProvides::Snippets => "Snippets",
        ExtensionProvides::DebugAdapters => "Debug Adapters",
    }
}

impl Component for ExtensionCard {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "A card that displays an extension's details, installation state, and available actions."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        fn local_extension(
            id: &'static str,
            name: &'static str,
            description: &'static str,
        ) -> Arc<ExtensionManifest> {
            Arc::new(ExtensionManifest {
                id: id.into(),
                name: name.to_owned(),
                version: "0.1.0".into(),
                schema_version: SchemaVersion::ZERO,
                description: Some(description.to_owned()),
                repository: Some("https://github.com/zed-industries/zed".to_owned()),
                authors: vec!["Extension Developer".to_owned()],
                lib: Default::default(),
                themes: Vec::new(),
                icon_themes: Vec::new(),
                languages: Vec::new(),
                grammars: Default::default(),
                language_servers: Default::default(),
                snippets: None,
                capabilities: Vec::new(),
                debug_adapters: Default::default(),
                debug_locators: Default::default(),
            })
        }

        let examples = vec![
            single_example(
                "Installed",
                ExtensionCard::manifest(
                    local_extension(
                        "preview-python",
                        "Python",
                        "Python language support powered by basedpyright.",
                    ),
                    ExtensionStatus::Installed("0.5.1".into()),
                    LocalExtensionKind::Installed,
                )
                .into_any_element(),
            ),
            single_example(
                "Development Extension",
                ExtensionCard::manifest(
                    local_extension(
                        "zed-demo-theme",
                        "Local Theme",
                        "A locally installed extension under development.",
                    ),
                    ExtensionStatus::Installed("0.1.0".into()),
                    LocalExtensionKind::Dev,
                )
                .into_any_element(),
            ),
        ];

        div()
            .w_128()
            .child(example_group(examples).vertical())
            .into_any_element()
    }
}

impl RenderOnce for ExtensionCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { details, actions } = self;
        let ExtensionCardDetails {
            name,
            version,
            description,
            authors,
            repository_url,
            repository_icon,
            provided_features,
            source,
        } = details;
        let is_dev = source.is_dev();
        let repository_button_id = SharedString::from(format!("repository-{name}"));

        div().w_full().child(
            v_flex()
                .mt_4()
                .w_full()
                .h(rems_from_px(110_f32))
                .p_3()
                .gap_2()
                .bg(cx.theme().colors().elevated_surface_background.opacity(0.5))
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .rounded_md()
                .child(
                    h_flex()
                        .gap_2()
                        .justify_between()
                        .child(
                            h_flex()
                                .flex_shrink_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .gap_2()
                                .child(Headline::new(name).size(HeadlineSize::Small))
                                .child(
                                    Headline::new(if is_dev {
                                        format!("v{version} (dev)")
                                    } else {
                                        format!("v{version}")
                                    })
                                    .size(HeadlineSize::XSmall)
                                    .color(Color::Muted),
                                )
                                .when(!provided_features.is_empty(), |parent| {
                                    parent.child(
                                        h_flex()
                                            .gap_1()
                                            .children(provided_features.into_iter().map(Chip::new)),
                                    )
                                }),
                        )
                        .child(
                            h_flex()
                                .flex_shrink_0()
                                .gap_1()
                                .children(actions.into_iter().flatten()),
                        ),
                )
                .child(h_flex().gap_2().justify_between().children(description.map(
                    |description| {
                        Label::new(description)
                            .size(LabelSize::Small)
                            .color(Color::Default)
                            .truncate()
                    },
                )))
                .child(
                    h_flex()
                        .min_w_0()
                        .w_full()
                        .justify_between()
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    Icon::new(IconName::Person)
                                        .size(IconSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    Label::new(authors)
                                        .size(LabelSize::Small)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        )
                        .child(h_flex().gap_1().flex_shrink_0().when_some(
                            repository_url,
                            |this, repository_url| {
                                let repository_url_for_tooltip = repository_url.clone();
                                this.child(
                                    IconButton::new(repository_button_id, repository_icon)
                                        .icon_size(IconSize::Small)
                                        .tooltip(move |_, cx| {
                                            Tooltip::with_meta(
                                                "Visit Extension Repository",
                                                None,
                                                repository_url_for_tooltip.clone(),
                                                cx,
                                            )
                                        })
                                        .on_click(move |_, _, cx| {
                                            cx.open_url(&repository_url);
                                        }),
                                )
                            },
                        )),
                ),
        )
    }
}
