use crate::{
    CloseButton, CloseButtonSize, DIALOG_METRICS, IconButtonShape, component_corner_radii,
    prelude::*,
};

use gpui::{prelude::FluentBuilder, *};
use smallvec::SmallVec;
use theme::ActiveTheme;

#[derive(IntoElement)]
pub struct Modal {
    id: ElementId,
    header: ModalHeader,
    children: SmallVec<[AnyElement; 2]>,
    footer: Option<ModalFooter>,
    container_id: ElementId,
    container_scroll_handler: Option<ScrollHandle>,
}

impl Modal {
    pub fn new(id: impl Into<SharedString>, scroll_handle: Option<ScrollHandle>) -> Self {
        let id = id.into();

        let container_id = ElementId::Name(format!("{}_container", id).into());
        Self {
            id: ElementId::Name(id),
            header: ModalHeader::new(),
            children: SmallVec::new(),
            footer: None,
            container_id,
            container_scroll_handler: scroll_handle,
        }
    }

    pub fn header(mut self, header: ModalHeader) -> Self {
        self.header = header;
        self
    }

    pub fn section(mut self, section: Section) -> Self {
        self.children.push(section.into_any_element());
        self
    }

    pub fn footer(mut self, footer: ModalFooter) -> Self {
        self.footer = Some(footer);
        self
    }

    pub fn show_dismiss(mut self, show: bool) -> Self {
        self.header.show_dismiss_button = show;
        self
    }

    pub fn show_back(mut self, show: bool) -> Self {
        self.header.show_back_button = show;
        self
    }
}

impl ParentElement for Modal {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl RenderOnce for Modal {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        v_flex()
            .id(self.id.clone())
            .size_full()
            .flex_1()
            .overflow_hidden()
            .child(self.header)
            .child(
                v_flex()
                    .id(self.container_id.clone())
                    .w_full()
                    .flex_1()
                    .gap(DIALOG_METRICS.section_gap)
                    .when_some(
                        self.container_scroll_handler,
                        |this, container_scroll_handle| {
                            this.overflow_y_scroll()
                                .track_scroll(&container_scroll_handle)
                        },
                    )
                    .children(self.children),
            )
            .children(self.footer)
    }
}

#[derive(IntoElement)]
pub struct ModalHeader {
    icon: Option<Icon>,
    headline: Option<SharedString>,
    description: Option<SharedString>,
    children: SmallVec<[AnyElement; 2]>,
    show_dismiss_button: bool,
    show_back_button: bool,
}

impl Default for ModalHeader {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalHeader {
    pub fn new() -> Self {
        Self {
            icon: None,
            headline: None,
            description: None,
            children: SmallVec::new(),
            show_dismiss_button: false,
            show_back_button: false,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set the headline of the modal.
    ///
    /// This will insert the headline as the first item
    /// of `children` if it is not already present.
    pub fn headline(mut self, headline: impl Into<SharedString>) -> Self {
        self.headline = Some(headline.into());
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn show_dismiss_button(mut self, show: bool) -> Self {
        self.show_dismiss_button = show;
        self
    }

    pub fn show_back_button(mut self, show: bool) -> Self {
        self.show_back_button = show;
        self
    }
}

impl ParentElement for ModalHeader {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl RenderOnce for ModalHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut children = self.children;

        if let Some(headline) = self.headline {
            children.insert(
                0,
                Label::new(headline)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .truncate()
                    .into_any_element(),
            );
        }

        h_flex()
            .min_w_0()
            .flex_none()
            .justify_between()
            .w_full()
            .px(DIALOG_METRICS.padding_x)
            .pt(DIALOG_METRICS.header_padding_top)
            .pb(DIALOG_METRICS.header_padding_bottom)
            .gap(DIALOG_METRICS.title_icon_gap)
            .when(self.show_back_button, |this| {
                this.child(
                    IconButton::new("back", IconName::ArrowLeft)
                        .shape(IconButtonShape::Square)
                        .on_click(|_, window, cx| {
                            window.dispatch_action(menu::Cancel.boxed_clone(), cx);
                        }),
                )
            })
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .child(
                        h_flex()
                            .w_full()
                            .gap(DIALOG_METRICS.title_icon_gap)
                            .justify_between()
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .gap(DIALOG_METRICS.title_icon_gap)
                                    .when_some(self.icon, |this, icon| this.child(icon))
                                    .children(children),
                            )
                            .when(self.show_dismiss_button, |this| {
                                this.child(
                                    CloseButton::new("dismiss", CloseButtonSize::Header).on_click(
                                        |_, window, cx| {
                                            window.dispatch_action(menu::Cancel.boxed_clone(), cx);
                                        },
                                    ),
                                )
                            }),
                    )
                    .when_some(self.description, |this, description| {
                        this.child(Label::new(description).color(Color::Muted).mb_2().flex_1())
                    }),
            )
    }
}

#[derive(IntoElement)]
pub struct ModalRow {
    children: SmallVec<[AnyElement; 2]>,
}

impl Default for ModalRow {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalRow {
    pub fn new() -> Self {
        Self {
            children: SmallVec::new(),
        }
    }
}

impl ParentElement for ModalRow {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl RenderOnce for ModalRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        h_flex().w_full().py_1().children(self.children)
    }
}

/// The button row of a dialog: an optional start slot (a checkbox, a link) and the dialog
/// buttons at the end, default button last, as on macOS and in IntelliJ.
#[derive(IntoElement)]
pub struct ModalFooter {
    start_slot: Option<AnyElement>,
    end_slot: Option<AnyElement>,
    end_buttons: SmallVec<[AnyElement; 3]>,
}

impl Default for ModalFooter {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalFooter {
    pub fn new() -> Self {
        Self {
            start_slot: None,
            end_slot: None,
            end_buttons: SmallVec::new(),
        }
    }

    pub fn start_slot<E: IntoElement>(mut self, start_slot: impl Into<Option<E>>) -> Self {
        self.start_slot = start_slot.into().map(IntoElement::into_any_element);
        self
    }

    /// Custom content for the end of the row. Dialog buttons go through
    /// [`Self::end_button`], which keeps the standard gap between them.
    pub fn end_slot<E: IntoElement>(mut self, end_slot: impl Into<Option<E>>) -> Self {
        self.end_slot = end_slot.into().map(IntoElement::into_any_element);
        self
    }

    pub fn end_button(mut self, button: impl IntoElement) -> Self {
        self.end_buttons.push(button.into_any_element());
        self
    }
}

impl RenderOnce for ModalFooter {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        h_flex()
            .debug_selector(|| "DIALOG_FOOTER".to_string())
            .w_full()
            .px(DIALOG_METRICS.padding_x)
            .pt(DIALOG_METRICS.footer_padding_top)
            .pb(DIALOG_METRICS.footer_padding_bottom)
            .flex_none()
            .justify_between()
            .gap(DIALOG_METRICS.button_group_gap)
            .child(
                h_flex()
                    .min_w_0()
                    .when_some(self.start_slot, |this, start_slot| this.child(start_slot)),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(DIALOG_METRICS.button_gap)
                    .when_some(self.end_slot, |this, end_slot| this.child(end_slot))
                    .children(self.end_buttons),
            )
    }
}

#[derive(IntoElement)]
pub struct Section {
    contained: bool,
    padded: bool,
    header: Option<SectionHeader>,
    meta: Option<SharedString>,
    children: SmallVec<[AnyElement; 2]>,
}

impl Default for Section {
    fn default() -> Self {
        Self::new()
    }
}

impl Section {
    pub fn new() -> Self {
        Self {
            contained: false,
            padded: true,
            header: None,
            meta: None,
            children: SmallVec::new(),
        }
    }

    pub fn new_contained() -> Self {
        Self {
            contained: true,
            padded: true,
            header: None,
            meta: None,
            children: SmallVec::new(),
        }
    }

    pub fn contained(mut self, contained: bool) -> Self {
        self.contained = contained;
        self
    }

    pub fn header(mut self, header: SectionHeader) -> Self {
        self.header = Some(header);
        self
    }

    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }
    pub fn padded(mut self, padded: bool) -> Self {
        self.padded = padded;
        self
    }
}

impl ParentElement for Section {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl RenderOnce for Section {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut section_bg = cx.theme().colors().text;
        section_bg.fade_out(0.96);

        let children = if self.contained {
            v_flex()
                .flex_1()
                .when(self.padded, |this| this.px(DIALOG_METRICS.padding_x))
                .child(
                    v_flex()
                        .w_full()
                        .rounded(component_corner_radii(cx).banner)
                        .border_1()
                        .border_color(cx.theme().colors().border)
                        .bg(section_bg)
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .pb_2()
                                .size_full()
                                .children(self.children),
                        ),
                )
        } else {
            v_flex()
                .w_full()
                .flex_1()
                .gap_y(DynamicSpacing::Base04.rems(cx))
                .pb_2()
                .when(self.padded, |this| this.px(DIALOG_METRICS.padding_x))
                .children(self.children)
        };

        v_flex()
            .size_full()
            .flex_1()
            .child(
                v_flex()
                    .flex_none()
                    .px(DIALOG_METRICS.padding_x)
                    .children(self.header)
                    .when_some(self.meta, |this, meta| {
                        this.child(Label::new(meta).size(LabelSize::Small).color(Color::Muted))
                    }),
            )
            .child(children)
            // fill any leftover space
            .child(div().flex().flex_1())
    }
}

#[derive(IntoElement)]
pub struct SectionHeader {
    /// The label of the header.
    label: SharedString,
    /// A slot for content that appears after the label, usually on the other side of the header.
    /// This might be a button, a disclosure arrow, a face pile, etc.
    end_slot: Option<AnyElement>,
}

impl SectionHeader {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            end_slot: None,
        }
    }

    pub fn end_slot<E: IntoElement>(mut self, end_slot: impl Into<Option<E>>) -> Self {
        self.end_slot = end_slot.into().map(IntoElement::into_any_element);
        self
    }
}

impl RenderOnce for SectionHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .id(self.label.clone())
            .w_full()
            .px(DynamicSpacing::Base08.rems(cx))
            .child(
                div()
                    .h_7()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .gap(DynamicSpacing::Base04.rems(cx))
                    .child(
                        div().flex_1().child(
                            Label::new(self.label.clone())
                                .size(LabelSize::Small)
                                .into_element(),
                        ),
                    )
                    .child(h_flex().children(self.end_slot)),
            )
    }
}

impl From<SharedString> for SectionHeader {
    fn from(val: SharedString) -> Self {
        SectionHeader::new(val)
    }
}

impl From<&'static str> for SectionHeader {
    fn from(val: &'static str) -> Self {
        let label: SharedString = val.into();
        SectionHeader::new(label)
    }
}
