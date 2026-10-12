use std::{cmp::Ordering, ops::Range};

use collections::HashMap;
use gpui::{
    AnyView, Bounds, ContentMask, Font, Hsla, PathBuilder, Point, ShapedLine, Size, TextAlign,
    TextRun, TransformationMatrix, canvas, fill, point, rgba, size,
};
use theme::Appearance;
use ui::{prelude::*, tooltip_container};
use util::ResultExt;

use crate::git_log::{graph::natural_compare, properties::HEAD_REVISION};

const TAG_DECORATION_PREFIX: &str = "tag: ";
const SYMBOLIC_REF_SEPARATOR: &str = " -> ";
const OTHER_REF_PREFIX: &str = "refs/";
const ORIGIN_HEAD: &str = "origin/HEAD";
const MASTER: &str = "master";
const MAIN: &str = "main";
const ORIGIN_MASTER: &str = "origin/master";
const ORIGIN_MAIN: &str = "origin/main";
const REMOTE_TABLE_SEPARATOR: &str = " & ";
const SEPARATOR: char = '/';
const DETACHED_HEAD_GROUP_NAME: &str = "Detached HEAD";

pub(crate) const DETACHED_HEAD_TOOLTIP: &str = "New commits may be lost because the current commit is not on any branch.\nCheck out a branch to avoid that.";

const TOP_TEXT_PADDING: Pixels = px(1.);
const BOTTOM_TEXT_PADDING: Pixels = px(2.);
const RIGHT_PADDING: Pixels = px(4.);
const LEFT_PADDING: Pixels = px(4.);
const COMPACT_MIDDLE_PADDING: Pixels = px(6.);
const MIDDLE_PADDING: Pixels = px(12.);
const ICON_TEXT_PADDING: Pixels = px(1.);
const ICON_ADDITIONAL_PADDING: Pixels = px(3.);
const WARNING_ICON_SIZE: Pixels = px(16.);
const MAX_LENGTH: usize = 22;
const TWO_DOTS: &str = "..";
const ELLIPSIS: &str = "\u{2026}";

const LABEL_ICON_SIZE: f32 = 6.25;
const TAG_ICON_SIZE: f32 = 16.0;
const TAG_STROKE_WIDTH: f32 = 1.0;
const TAG_DOT_RADIUS: f32 = 1.0;
const TAG_DOT_CENTER: (f32, f32) = (10.5, 5.5);
const TAG_OUTLINE_START: (f32, f32) = (13.4495, 7.5001);
const TAG_OUTLINE: [TagSegment; 10] = [
    TagSegment::Line(7.01028, 13.9394),
    TagSegment::Curve([6.81502, 14.1347, 6.49843, 14.1347, 6.30317, 13.9394]),
    TagSegment::Line(2.06053, 9.69679),
    TagSegment::Curve([1.86527, 9.50152, 1.86527, 9.18494, 2.06053, 8.98968]),
    TagSegment::Line(8.49978, 2.55035),
    TagSegment::Curve([8.61106, 2.43907, 8.76824, 2.38667, 8.92404, 2.40893]),
    TagSegment::Line(12.6363, 2.93926),
    TagSegment::Curve([12.8563, 2.97069, 13.0292, 3.14354, 13.0606, 3.36352]),
    TagSegment::Line(13.5909, 7.07584),
    TagSegment::Curve([13.6132, 7.23163, 13.5608, 7.38882, 13.4495, 7.5001]),
];

const REFS_LIMIT: usize = 10;
const TOOLTIP_HORIZONTAL_GAP: Pixels = px(4.);
const TOOLTIP_ICON_TEXT_GAP: Pixels = px(2.);
const REFERENCE_FONT_SIZE_DECREMENT: Pixels = px(1.);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RefLabelKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
    Other,
}

impl RefLabelKind {
    fn is_branch(self) -> bool {
        matches!(self, Self::Head | Self::LocalBranch | Self::RemoteBranch)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefLabel {
    pub name: SharedString,
    pub kind: RefLabelKind,
}

pub(crate) fn parse_ref_labels(
    ref_names: &[SharedString],
    remote_names: &[SharedString],
) -> Vec<RefLabel> {
    let mut labels = Vec::new();
    for decoration in ref_names {
        if let Some(tag) = decoration.strip_prefix(TAG_DECORATION_PREFIX) {
            push_ref_label(&mut labels, tag.trim(), RefLabelKind::Tag);
        } else {
            for name in decoration.split(SYMBOLIC_REF_SEPARATOR).map(str::trim) {
                push_ref_label(&mut labels, name, ref_label_kind(name, remote_names));
            }
        }
    }
    labels
}

fn push_ref_label(labels: &mut Vec<RefLabel>, name: &str, kind: RefLabelKind) {
    if !name.is_empty() && name != ORIGIN_HEAD {
        labels.push(RefLabel {
            name: SharedString::new(name),
            kind,
        });
    }
}

fn ref_label_kind(name: &str, remote_names: &[SharedString]) -> RefLabelKind {
    if name == HEAD_REVISION {
        RefLabelKind::Head
    } else if name.starts_with(OTHER_REF_PREFIX) {
        RefLabelKind::Other
    } else if name.split_once(SEPARATOR).is_some_and(|(remote, _)| {
        remote_names
            .iter()
            .any(|remote_name| remote_name.as_ref() == remote)
    }) {
        RefLabelKind::RemoteBranch
    } else {
        RefLabelKind::LocalBranch
    }
}

struct RefIconColors {
    head: u32,
    local_branch: u32,
    remote_branch: u32,
    tag: u32,
    other: u32,
}

const ISLANDS_DARK_REF_ICON_COLORS: RefIconColors = RefIconColors {
    head: 0xF5D273FF,
    local_branch: 0x5FAD65FF,
    remote_branch: 0xB589ECFF,
    tag: 0x868A91FF,
    other: 0x868A91FF,
};

const ISLANDS_LIGHT_REF_ICON_COLORS: RefIconColors = RefIconColors {
    head: 0xFFAF0FFF,
    local_branch: 0x369650FF,
    remote_branch: 0x834DF0FF,
    tag: 0x6C707EFF,
    other: 0x6C707EFF,
};

pub(crate) fn ref_label_color(kind: RefLabelKind, cx: &App) -> Hsla {
    let colors = match cx.theme().appearance() {
        Appearance::Light => &ISLANDS_LIGHT_REF_ICON_COLORS,
        Appearance::Dark => &ISLANDS_DARK_REF_ICON_COLORS,
    };
    let color = match kind {
        RefLabelKind::Head => colors.head,
        RefLabelKind::LocalBranch => colors.local_branch,
        RefLabelKind::RemoteBranch => colors.remote_branch,
        RefLabelKind::Tag => colors.tag,
        RefLabelKind::Other => colors.other,
    };
    rgba(color).into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LabelOrderType {
    Head,
    CurrentBranch,
    Master,
    OriginMaster,
    LocalBranch,
    RemoteBranch,
    Tag,
    Other,
}

fn label_order_type(label: &RefLabel, current_branch: Option<&str>) -> LabelOrderType {
    let name = label.name.as_ref();
    match label.kind {
        RefLabelKind::Head => LabelOrderType::Head,
        RefLabelKind::Tag => LabelOrderType::Tag,
        RefLabelKind::LocalBranch if current_branch == Some(name) => LabelOrderType::CurrentBranch,
        RefLabelKind::LocalBranch if name == MASTER || name == MAIN => LabelOrderType::Master,
        RefLabelKind::LocalBranch => LabelOrderType::LocalBranch,
        RefLabelKind::RemoteBranch if name == ORIGIN_MASTER || name == ORIGIN_MAIN => {
            LabelOrderType::OriginMaster
        }
        RefLabelKind::RemoteBranch => LabelOrderType::RemoteBranch,
        RefLabelKind::Other => LabelOrderType::Other,
    }
}

pub(crate) fn compare_ref_labels(
    left: &RefLabel,
    right: &RefLabel,
    current_branch: Option<&str>,
) -> Ordering {
    label_order_type(left, current_branch)
        .cmp(&label_order_type(right, current_branch))
        .then_with(|| natural_compare(&left.name, &right.name))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RefGroupingContext {
    pub current_branch: Option<SharedString>,
    pub upstreams: HashMap<SharedString, SharedString>,
    pub remote_names: Vec<SharedString>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefLabelGroup {
    pub name: SharedString,
    pub labels: Vec<RefLabel>,
    pub is_detached_head: bool,
}

impl RefLabelGroup {
    pub(crate) fn icon_kinds(&self) -> Vec<RefLabelKind> {
        if self.is_detached_head {
            return Vec::new();
        }
        doubled_when_repeated(self.labels.iter().map(|label| label.kind))
    }
}

fn doubled_when_repeated<T: Copy + PartialEq>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut counts: Vec<(T, usize)> = Vec::new();
    for item in items {
        match counts.iter_mut().find(|(existing, _)| *existing == item) {
            Some((_, count)) => *count += 1,
            None => counts.push((item, 1)),
        }
    }
    let mut result = Vec::with_capacity(counts.len() * 2);
    for (item, count) in counts {
        result.push(item);
        if count > 1 {
            result.push(item);
        }
    }
    result
}

fn simple_group(name: SharedString, labels: Vec<RefLabel>) -> RefLabelGroup {
    RefLabelGroup {
        name,
        labels,
        is_detached_head: false,
    }
}

type KindGroups = Vec<(RefLabelKind, Vec<RefLabel>)>;

pub(crate) fn group_ref_labels(
    labels: &[RefLabel],
    context: &RefGroupingContext,
    compact: bool,
    show_tag_names: bool,
) -> Vec<RefLabelGroup> {
    if labels.is_empty() {
        return Vec::new();
    }
    let current_branch = context.current_branch.as_deref();
    let mut sorted_labels = labels.to_vec();
    sorted_labels.sort_by(|left, right| compare_ref_labels(left, right, current_branch));
    let mut grouped = group_by_kind(sorted_labels);
    let head_labels = take_kind(&mut grouped, RefLabelKind::Head);

    let tracked = tracked_groups(&grouped, context);
    for group in &tracked {
        if let [local, remote] = group.labels.as_slice() {
            remove_label(&mut grouped, local);
            remove_label(&mut grouped, remote);
        }
    }

    let current_branch_group = context.current_branch.as_ref().and_then(|branch_name| {
        let label = take_local_branch(&mut grouped, branch_name)?;
        Some(simple_group(branch_name.clone(), vec![label]))
    });

    let leading_groups = current_branch_group.into_iter().chain(tracked).collect();
    let mut result = build_groups(leading_groups, grouped, compact, show_tag_names);
    if head_labels.is_empty() {
        return result;
    }
    if context.current_branch.is_none() {
        result.insert(
            0,
            RefLabelGroup {
                name: SharedString::new_static(DETACHED_HEAD_GROUP_NAME),
                labels: head_labels,
                is_detached_head: true,
            },
        );
    } else if let Some(first_group) = result.first_mut() {
        first_group.labels = head_labels
            .into_iter()
            .chain(std::mem::take(&mut first_group.labels))
            .collect();
    } else {
        result.insert(0, simple_group(SharedString::default(), head_labels));
    }
    result
}

fn group_by_kind(labels: impl IntoIterator<Item = RefLabel>) -> KindGroups {
    let mut groups: KindGroups = Vec::new();
    for label in labels {
        match groups.iter_mut().find(|(kind, _)| *kind == label.kind) {
            Some((_, members)) => members.push(label),
            None => groups.push((label.kind, vec![label])),
        }
    }
    groups
}

fn take_kind(groups: &mut KindGroups, kind: RefLabelKind) -> Vec<RefLabel> {
    match groups
        .iter()
        .position(|(group_kind, _)| *group_kind == kind)
    {
        Some(index) => groups.remove(index).1,
        None => Vec::new(),
    }
}

fn labels_of_kind(groups: &KindGroups, kind: RefLabelKind) -> &[RefLabel] {
    groups
        .iter()
        .find(|(group_kind, _)| *group_kind == kind)
        .map(|(_, labels)| labels.as_slice())
        .unwrap_or_default()
}

fn remove_label(groups: &mut KindGroups, label: &RefLabel) {
    let Some(group_index) = groups
        .iter()
        .position(|(group_kind, _)| *group_kind == label.kind)
    else {
        return;
    };
    let Some((_, members)) = groups.get_mut(group_index) else {
        return;
    };
    if let Some(member_index) = members.iter().position(|member| member == label) {
        members.remove(member_index);
    }
    if members.is_empty() {
        groups.remove(group_index);
    }
}

fn take_local_branch(groups: &mut KindGroups, name: &str) -> Option<RefLabel> {
    let (_, members) = groups
        .iter_mut()
        .find(|(kind, _)| *kind == RefLabelKind::LocalBranch)?;
    let index = members
        .iter()
        .position(|member| member.name.as_ref() == name)?;
    Some(members.remove(index))
}

fn tracked_groups(groups: &KindGroups, context: &RefGroupingContext) -> Vec<RefLabelGroup> {
    let remotes = labels_of_kind(groups, RefLabelKind::RemoteBranch);
    labels_of_kind(groups, RefLabelKind::LocalBranch)
        .iter()
        .filter_map(|local| tracked_group(local, remotes, context))
        .collect()
}

fn tracked_group(
    local: &RefLabel,
    remotes: &[RefLabel],
    context: &RefGroupingContext,
) -> Option<RefLabelGroup> {
    if let Some(upstream) = context.upstreams.get(&local.name)
        && let Some(tracked) = remotes.iter().find(|remote| remote.name == *upstream)
        && let Some((remote_name, _)) = upstream.split_once(SEPARATOR)
    {
        return Some(tracked_pair(remote_name, local, tracked));
    }
    context.remote_names.iter().find_map(|remote_name| {
        let candidate_name = format!("{remote_name}{SEPARATOR}{}", local.name);
        remotes
            .iter()
            .find(|remote| remote.name.as_ref() == candidate_name)
            .map(|remote| tracked_pair(remote_name, local, remote))
    })
}

fn tracked_pair(remote_name: &str, local: &RefLabel, remote: &RefLabel) -> RefLabelGroup {
    simple_group(
        SharedString::from(format!(
            "{remote_name}{REMOTE_TABLE_SEPARATOR}{}",
            local.name
        )),
        vec![local.clone(), remote.clone()],
    )
}

fn build_groups(
    groups: Vec<RefLabelGroup>,
    grouped: KindGroups,
    compact: bool,
    show_tag_names: bool,
) -> Vec<RefLabelGroup> {
    let remaining: Vec<RefLabel> = grouped
        .iter()
        .flat_map(|(_, labels)| labels.iter().cloned())
        .collect();
    let Some(first_remaining) = remaining.first() else {
        return groups;
    };
    if compact {
        if let Some(first_group) = groups.first() {
            let name = first_group.name.clone();
            let labels = groups
                .into_iter()
                .flat_map(|group| group.labels)
                .chain(remaining)
                .collect();
            return vec![simple_group(name, labels)];
        }
        let name = if first_remaining.kind.is_branch() || show_tag_names {
            first_remaining.name.clone()
        } else {
            SharedString::default()
        };
        return vec![simple_group(name, remaining)];
    }
    let mut result = groups;
    for (kind, labels) in grouped {
        if kind.is_branch() {
            result.extend(
                labels
                    .into_iter()
                    .map(|label| simple_group(label.name.clone(), vec![label])),
            );
        } else if let Some(first_label) = labels.first() {
            let name = if show_tag_names {
                first_label.name.clone()
            } else {
                SharedString::default()
            };
            result.push(simple_group(name, labels));
        }
    }
    result
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RefLabelsStyle {
    pub font: Font,
    pub font_size: Pixels,
    pub compact: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RefLabelsLayout {
    labels: Vec<LabelPresentation>,
    width: Pixels,
    font_height: Pixels,
    ascent: Pixels,
    compact: bool,
    warning_range: Option<Range<Pixels>>,
}

impl RefLabelsLayout {
    pub(crate) fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    pub(crate) fn width(&self) -> Pixels {
        self.width
    }

    pub(crate) fn height(&self) -> Pixels {
        if self.labels.is_empty() {
            Pixels::ZERO
        } else {
            self.font_height + TOP_TEXT_PADDING + BOTTOM_TEXT_PADDING
        }
    }

    pub(crate) fn warning_range(&self) -> Option<Range<Pixels>> {
        self.warning_range.clone()
    }
}

#[derive(Clone, Debug)]
struct LabelPresentation {
    text: ShapedLine,
    icon: LabelPresentationIcon,
    width: Pixels,
    icon_padding: Pixels,
}

impl LabelPresentation {
    fn new(
        text: ShapedLine,
        icon: LabelPresentationIcon,
        icon_width: Pixels,
        icon_padding: Pixels,
    ) -> Self {
        let width = icon_width + icon_padding + padded_text_width(text.width());
        Self {
            text,
            icon,
            width,
            icon_padding,
        }
    }
}

#[derive(Clone, Debug)]
enum LabelPresentationIcon {
    Tags(Vec<Hsla>),
    Warning(Hsla),
}

impl LabelPresentationIcon {
    fn size(&self, font_height: Pixels) -> Size<Pixels> {
        match self {
            Self::Tags(colors) => size(label_icon_width(font_height, colors.len()), font_height),
            Self::Warning(_) => size(WARNING_ICON_SIZE, WARNING_ICON_SIZE),
        }
    }
}

#[derive(Debug, PartialEq)]
struct PlannedLabel {
    text: SharedString,
    colors: Vec<Hsla>,
}

fn middle_padding(compact: bool) -> Pixels {
    if compact {
        COMPACT_MIDDLE_PADDING
    } else {
        MIDDLE_PADDING
    }
}

fn padded_text_width(text_width: Pixels) -> Pixels {
    if text_width > Pixels::ZERO {
        ICON_TEXT_PADDING + text_width
    } else {
        Pixels::ZERO
    }
}

fn font_metrics(font: &Font, font_size: Pixels, window: &Window) -> (Pixels, Pixels) {
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(font);
    let ascent = text_system.ascent(font_id, font_size);
    let font_height = ascent + text_system.descent(font_id, font_size).abs();
    (ascent, font_height)
}

pub(crate) fn layout_ref_labels(
    groups: &[RefLabelGroup],
    available_width: Pixels,
    style: &RefLabelsStyle,
    foreground: Hsla,
    window: &Window,
    cx: &App,
) -> RefLabelsLayout {
    let (ascent, font_height) = font_metrics(&style.font, style.font_size, window);
    let mut layout = RefLabelsLayout {
        labels: Vec::new(),
        width: Pixels::ZERO,
        font_height,
        ascent,
        compact: style.compact,
        warning_range: None,
    };
    let Some(first_group) = groups.first() else {
        return layout;
    };
    let text_system = window.text_system();
    let shape = |text: SharedString, color: Hsla| {
        let runs = [TextRun {
            len: text.len(),
            font: style.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        text_system.shape_line(text, style.font_size, &runs, None)
    };
    let measure = |text: &str| shape(SharedString::new(text), foreground).width();
    let middle_padding = middle_padding(style.compact);

    let detached_head = first_group.is_detached_head.then(|| {
        let warning = cx.theme().status().warning;
        LabelPresentation::new(
            shape(first_group.name.clone(), warning),
            LabelPresentationIcon::Warning(warning),
            WARNING_ICON_SIZE,
            ICON_ADDITIONAL_PADDING,
        )
    });
    let detached_head_width = detached_head
        .as_ref()
        .map_or(Pixels::ZERO, |label| label.width + middle_padding);
    let remaining_width = available_width - detached_head_width - (LEFT_PADDING + RIGHT_PADDING);
    let group_colors: Vec<Vec<Hsla>> = groups
        .iter()
        .map(|group| {
            group
                .icon_kinds()
                .into_iter()
                .map(|kind| ref_label_color(kind, cx))
                .collect()
        })
        .collect();
    let planned_labels = plan_reference_labels(
        groups,
        &group_colors,
        remaining_width,
        font_height,
        style.compact,
        &measure,
    );

    if let Some(detached_head) = detached_head {
        layout.warning_range = Some(LEFT_PADDING..LEFT_PADDING + detached_head.width);
        layout.labels.push(detached_head);
    }
    for planned_label in planned_labels {
        let icon_width = label_icon_width(font_height, planned_label.colors.len());
        layout.labels.push(LabelPresentation::new(
            shape(planned_label.text, foreground),
            LabelPresentationIcon::Tags(planned_label.colors),
            icon_width,
            Pixels::ZERO,
        ));
    }
    let labels_width = presentations_width(&layout.labels, middle_padding);
    layout.width = if labels_width > Pixels::ZERO {
        labels_width + LEFT_PADDING + RIGHT_PADDING
    } else {
        Pixels::ZERO
    };
    layout
}

fn presentations_width(labels: &[LabelPresentation], middle_padding: Pixels) -> Pixels {
    if labels.is_empty() {
        return Pixels::ZERO;
    }
    labels.iter().map(|label| label.width).sum::<Pixels>() + middle_padding * (labels.len() - 1)
}

fn plan_reference_labels(
    groups: &[RefLabelGroup],
    group_colors: &[Vec<Hsla>],
    available_width: Pixels,
    font_height: Pixels,
    compact: bool,
    measure: &dyn Fn(&str) -> Pixels,
) -> Vec<PlannedLabel> {
    let middle_padding = middle_padding(compact);
    let mut labels = Vec::new();
    let mut references_width = Pixels::ZERO;
    for (index, (group, colors)) in groups.iter().zip(group_colors).enumerate() {
        if group.is_detached_head {
            continue;
        }
        let is_last = index + 1 == groups.len();
        let do_not_fit_width = if !compact && !is_last {
            label_icon_width(
                font_height,
                combined_colors(group_colors.get(index + 1..).unwrap_or_default()).len(),
            )
        } else {
            Pixels::ZERO
        };
        let mut new_width = references_width
            + label_icon_width(font_height, colors.len())
            + if is_last {
                Pixels::ZERO
            } else {
                middle_padding
            };
        let text = shorten_ref_name(
            &group.name,
            available_width - new_width - do_not_fit_width,
            measure,
        );
        new_width += padded_text_width(measure(text.as_ref()));
        if !compact && available_width - new_width - do_not_fit_width < Pixels::ZERO {
            let collapsed_text = if labels.is_empty() {
                text
            } else {
                SharedString::default()
            };
            labels.push(PlannedLabel {
                text: collapsed_text,
                colors: combined_colors(group_colors.get(index..).unwrap_or_default()),
            });
            break;
        }
        labels.push(PlannedLabel {
            text,
            colors: colors.clone(),
        });
        references_width = new_width;
    }
    labels
}

fn combined_colors(group_colors: &[Vec<Hsla>]) -> Vec<Hsla> {
    doubled_when_repeated(group_colors.iter().flatten().copied())
}

fn shorten_ref_name(
    ref_name: &SharedString,
    available_width: Pixels,
    measure: &dyn Fn(&str) -> Pixels,
) -> SharedString {
    if measure(ref_name.as_ref()) <= available_width || ref_name.chars().count() <= MAX_LENGTH {
        return ref_name.clone();
    }
    let mut name = ref_name.to_string();
    if let Some(separator_index) = name.find(SEPARATOR)
        && name[..separator_index].chars().count() > TWO_DOTS.len()
    {
        name = format!("{TWO_DOTS}{}", &name[separator_index..]);
    }
    if available_width > Pixels::ZERO {
        return shorten_text_to_fit(&name, available_width, measure).into();
    }
    shorten_text_with_ellipsis(&name, MAX_LENGTH).into()
}

fn shorten_text_to_fit(
    text: &str,
    available_width: Pixels,
    measure: &dyn Fn(&str) -> Pixels,
) -> String {
    if measure(text) <= available_width {
        return text.to_string();
    }
    for max_length in (MAX_LENGTH + 1..=text.chars().count()).rev() {
        let shortened = shorten_text_with_ellipsis(text, max_length);
        if measure(&shortened) <= available_width {
            return shortened;
        }
    }
    shorten_text_with_ellipsis(text, MAX_LENGTH)
}

fn shorten_text_with_ellipsis(text: &str, max_length: usize) -> String {
    if text.chars().count() <= max_length {
        return text.to_string();
    }
    let prefix_length = max_length.saturating_sub(ELLIPSIS.chars().count());
    let prefix_end = text
        .char_indices()
        .nth(prefix_length)
        .map_or(text.len(), |(index, _)| index);
    format!("{}{ELLIPSIS}", &text[..prefix_end])
}

fn label_icon_width(height: Pixels, labels_count: usize) -> Pixels {
    let scale = f32::from(height) / LABEL_ICON_SIZE;
    let width = (LABEL_ICON_SIZE + 2.0 * (labels_count as f32 - 1.0)) * scale;
    px((width + 0.5).floor())
}

pub(crate) fn paint_ref_labels(
    layout: &RefLabelsLayout,
    bounds: Bounds<Pixels>,
    background: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    if layout.labels.is_empty() {
        return;
    }
    let height = bounds.size.height;
    let baseline = ((height - layout.font_height + px(1.)) / 2.).floor() + layout.ascent;
    window.paint_quad(fill(
        Bounds::new(bounds.origin, size(layout.width, height)),
        background,
    ));
    let middle_padding = middle_padding(layout.compact);
    let mut x = bounds.origin.x + LEFT_PADDING;
    for label in &layout.labels {
        let icon_size = label.icon.size(layout.font_height);
        let icon_origin = point(
            x,
            bounds.origin.y + ((height - icon_size.height) / 2.).floor(),
        );
        match &label.icon {
            LabelPresentationIcon::Tags(colors) => {
                paint_label_icon(icon_origin, layout.font_height, colors, background, window);
            }
            LabelPresentationIcon::Warning(color) => {
                window
                    .paint_svg(
                        Bounds::new(icon_origin, icon_size),
                        IconName::Warning.path().into(),
                        None,
                        TransformationMatrix::unit(),
                        *color,
                        cx,
                    )
                    .log_err();
            }
        }
        x += icon_size.width + ICON_TEXT_PADDING + label.icon_padding;
        let text_origin = point(x, bounds.origin.y + baseline - label.text.ascent);
        label
            .text
            .paint(
                text_origin,
                label.text.ascent + label.text.descent,
                TextAlign::Left,
                None,
                window,
                cx,
            )
            .log_err();
        x += label.text.width() + middle_padding;
    }
}

pub(crate) fn label_icon(colors: Vec<Hsla>, height: Pixels, background: Hsla) -> AnyElement {
    let width = label_icon_width(height, colors.len());
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            paint_label_icon(bounds.origin, height, &colors, background, window)
        },
    )
    .flex_none()
    .w(width)
    .h(height)
    .into_any_element()
}

fn snap_to_device_pixel(value: Pixels, scale_factor: f32) -> Pixels {
    if scale_factor <= 0.0 {
        return value;
    }
    px((f32::from(value) * scale_factor).floor() / scale_factor)
}

fn paint_label_icon(
    origin: Point<Pixels>,
    height: Pixels,
    colors: &[Hsla],
    background: Hsla,
    window: &mut Window,
) {
    let scale_factor = window.scale_factor();
    let origin = point(
        snap_to_device_pixel(origin.x, scale_factor),
        snap_to_device_pixel(origin.y, scale_factor),
    );
    let bounds = Bounds::new(origin, size(label_icon_width(height, colors.len()), height));
    let scale = f32::from(height) / LABEL_ICON_SIZE;
    let tag_count = colors.len();
    window.with_content_mask(
        Some(ContentMask {
            bounds,
            ..Default::default()
        }),
        |window| {
            for (index, color) in colors.iter().rev().enumerate() {
                let offset = scale * 2.0 * index as f32;
                let is_front = index + 1 == tag_count;
                paint_tag(bounds, offset, is_front, background, *color, window);
                if !is_front {
                    let gap_offset = offset + 2.0 * scale - std::f32::consts::SQRT_2;
                    paint_tag(bounds, gap_offset, false, background, background, window);
                }
            }
        },
    );
}

#[derive(Clone, Copy)]
enum TagSegment {
    Line(f32, f32),
    Curve([f32; 6]),
}

fn paint_tag(
    bounds: Bounds<Pixels>,
    offset: f32,
    with_dot: bool,
    background: Hsla,
    tag_color: Hsla,
    window: &mut Window,
) {
    let scale = f32::from(bounds.size.height) / TAG_ICON_SIZE;
    let origin = bounds.origin;
    let at = |x: f32, y: f32| point(origin.x + px(x * scale + offset), origin.y + px(y * scale));
    paint_shape(
        tag_outline(PathBuilder::fill(), &at),
        background,
        bounds,
        window,
    );
    paint_shape(
        tag_outline(PathBuilder::stroke(px(TAG_STROKE_WIDTH * scale)), &at),
        tag_color,
        bounds,
        window,
    );
    if with_dot {
        paint_shape(
            circle(
                at(TAG_DOT_CENTER.0, TAG_DOT_CENTER.1),
                px(TAG_DOT_RADIUS * scale),
            ),
            tag_color,
            bounds,
            window,
        );
    }
}

fn tag_outline(mut builder: PathBuilder, at: &dyn Fn(f32, f32) -> Point<Pixels>) -> PathBuilder {
    builder.move_to(at(TAG_OUTLINE_START.0, TAG_OUTLINE_START.1));
    for segment in TAG_OUTLINE {
        match segment {
            TagSegment::Line(x, y) => builder.line_to(at(x, y)),
            TagSegment::Curve([first_x, first_y, second_x, second_y, end_x, end_y]) => builder
                .cubic_bezier_to(
                    at(end_x, end_y),
                    at(first_x, first_y),
                    at(second_x, second_y),
                ),
        }
    }
    builder.close();
    builder
}

fn circle(center: Point<Pixels>, radius: Pixels) -> PathBuilder {
    let radii = point(radius, radius);
    let mut builder = PathBuilder::fill();
    builder.move_to(point(center.x + radius, center.y));
    builder.arc_to(
        radii,
        px(0.),
        false,
        true,
        point(center.x - radius, center.y),
    );
    builder.arc_to(
        radii,
        px(0.),
        false,
        true,
        point(center.x + radius, center.y),
    );
    builder.close();
    builder
}

fn paint_shape(builder: PathBuilder, color: Hsla, bounds: Bounds<Pixels>, window: &mut Window) {
    if let Some(path) = builder.build().log_err() {
        window.paint_layer(bounds, |window| window.paint_path(path, color));
    }
}

pub(crate) fn references_tooltip(
    labels: Vec<RefLabel>,
    _window: &mut Window,
    cx: &mut App,
) -> AnyView {
    cx.new(|_| ReferencesTooltip { labels }).into()
}

struct ReferencesTooltip {
    labels: Vec<RefLabel>,
}

impl Render for ReferencesTooltip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_settings = theme::theme_settings(cx);
        let font = theme_settings.ui_font(cx).clone();
        let font_size = theme_settings.ui_font_size(cx) - REFERENCE_FONT_SIZE_DECREMENT;
        let (_, font_height) = font_metrics(&font, font_size, window);
        let colors = cx.theme().colors();
        let background = colors
            .elevated_surface_background
            .blend(colors.element_background);
        let rest_color = colors.text_disabled;

        let grouped = group_by_kind(self.labels.iter().take(REFS_LIMIT).cloned());
        let has_group_with_multiple_refs = grouped.iter().any(|(_, labels)| labels.len() > 1);
        let first_icon_width = label_icon_width(
            font_height,
            if has_group_with_multiple_refs { 2 } else { 1 },
        );
        let empty_icon_width = label_icon_width(font_height, 2);

        let mut rows = Vec::new();
        for (kind, labels) in grouped {
            let color = ref_label_color(kind, cx);
            let last_index = labels.len().saturating_sub(1);
            for (index, label) in labels.iter().enumerate() {
                let icon = if index == 0 {
                    let icon_colors = if labels.len() > 1 {
                        vec![color, color]
                    } else {
                        vec![color]
                    };
                    div()
                        .flex_none()
                        .w(first_icon_width)
                        .h(font_height)
                        .child(label_icon(icon_colors, font_height, background))
                } else {
                    div().flex_none().w(empty_icon_width).h(font_height)
                };
                let ending = if index == last_index { "" } else { "," };
                rows.push(reference_row(
                    icon,
                    format!("{}{ending}", label.name).into(),
                    None,
                ));
            }
        }
        let hidden_count = self.labels.len().saturating_sub(REFS_LIMIT);
        if hidden_count > 0 {
            rows.push(reference_row(
                div().flex_none().w(empty_icon_width).h(font_height),
                format!("... {hidden_count} more in details pane").into(),
                Some(rest_color),
            ));
        }

        tooltip_container(cx, move |container, _| {
            container.child(
                v_flex()
                    .px(TOOLTIP_HORIZONTAL_GAP)
                    .text_size(font_size)
                    .line_height(font_height)
                    .children(rows),
            )
        })
    }
}

fn reference_row(icon: Div, text: SharedString, color: Option<Hsla>) -> Div {
    h_flex().gap(TOOLTIP_ICON_TEXT_GAP).child(icon).child(
        div()
            .when_some(color, |this, color| this.text_color(color))
            .child(text),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn label(name: &'static str, kind: RefLabelKind) -> RefLabel {
        RefLabel {
            name: SharedString::new_static(name),
            kind,
        }
    }

    fn group(name: &'static str, labels: Vec<RefLabel>) -> RefLabelGroup {
        simple_group(SharedString::new_static(name), labels)
    }

    fn context(
        current_branch: Option<&'static str>,
        upstreams: &[(&'static str, &'static str)],
    ) -> RefGroupingContext {
        RefGroupingContext {
            current_branch: current_branch.map(SharedString::new_static),
            upstreams: upstreams
                .iter()
                .map(|(local, upstream)| {
                    (
                        SharedString::new_static(local),
                        SharedString::new_static(upstream),
                    )
                })
                .collect(),
            remote_names: vec![SharedString::new_static("origin")],
        }
    }

    fn ten_pixels_per_character(text: &str) -> Pixels {
        px(text.chars().count() as f32 * 10.)
    }

    fn shorten(name: &'static str, available_width: f32) -> SharedString {
        shorten_ref_name(
            &SharedString::new_static(name),
            px(available_width),
            &ten_pixels_per_character,
        )
    }

    #[test]
    fn parse_ref_labels_decodes_git_decorations() {
        let remotes = [SharedString::new_static("origin")];
        let decorations = [
            "HEAD -> main",
            "origin/main",
            "origin/HEAD",
            "tag: v1.0",
            "refs/stash",
            "feature/x",
        ]
        .map(SharedString::new_static);
        assert_eq!(
            parse_ref_labels(&decorations, &remotes),
            vec![
                label("HEAD", RefLabelKind::Head),
                label("main", RefLabelKind::LocalBranch),
                label("origin/main", RefLabelKind::RemoteBranch),
                label("v1.0", RefLabelKind::Tag),
                label("refs/stash", RefLabelKind::Other),
                label("feature/x", RefLabelKind::LocalBranch),
            ]
        );
        assert_eq!(
            parse_ref_labels(&[SharedString::new_static("HEAD")], &remotes),
            vec![label("HEAD", RefLabelKind::Head)]
        );
    }

    #[test]
    fn labels_are_ordered_like_the_git_label_comparator() {
        let mut labels = vec![
            label("v2", RefLabelKind::Tag),
            label("origin/feature", RefLabelKind::RemoteBranch),
            label("feature10", RefLabelKind::LocalBranch),
            label("refs/stash", RefLabelKind::Other),
            label("origin/main", RefLabelKind::RemoteBranch),
            label("feature2", RefLabelKind::LocalBranch),
            label("main", RefLabelKind::LocalBranch),
            label("current", RefLabelKind::LocalBranch),
            label("HEAD", RefLabelKind::Head),
        ];
        labels.sort_by(|left, right| compare_ref_labels(left, right, Some("current")));
        assert_eq!(
            labels,
            vec![
                label("HEAD", RefLabelKind::Head),
                label("current", RefLabelKind::LocalBranch),
                label("main", RefLabelKind::LocalBranch),
                label("origin/main", RefLabelKind::RemoteBranch),
                label("feature2", RefLabelKind::LocalBranch),
                label("feature10", RefLabelKind::LocalBranch),
                label("origin/feature", RefLabelKind::RemoteBranch),
                label("v2", RefLabelKind::Tag),
                label("refs/stash", RefLabelKind::Other),
            ]
        );
    }

    #[test]
    fn tracked_pairs_form_groups_and_head_joins_the_first_group() {
        let labels = [
            label("v1", RefLabelKind::Tag),
            label("feature", RefLabelKind::LocalBranch),
            label("origin/feature", RefLabelKind::RemoteBranch),
            label("origin/main", RefLabelKind::RemoteBranch),
            label("main", RefLabelKind::LocalBranch),
            label("HEAD", RefLabelKind::Head),
        ];
        assert_eq!(
            group_ref_labels(
                &labels,
                &context(Some("main"), &[("main", "origin/main")]),
                false,
                false
            ),
            vec![
                group(
                    "origin & main",
                    vec![
                        label("HEAD", RefLabelKind::Head),
                        label("main", RefLabelKind::LocalBranch),
                        label("origin/main", RefLabelKind::RemoteBranch),
                    ]
                ),
                group(
                    "origin & feature",
                    vec![
                        label("feature", RefLabelKind::LocalBranch),
                        label("origin/feature", RefLabelKind::RemoteBranch),
                    ]
                ),
                group("", vec![label("v1", RefLabelKind::Tag)]),
            ]
        );
    }

    #[test]
    fn current_branch_without_tracked_pair_gets_its_own_group() {
        let labels = [
            label("main", RefLabelKind::LocalBranch),
            label("v1", RefLabelKind::Tag),
            label("feature", RefLabelKind::LocalBranch),
            label("HEAD", RefLabelKind::Head),
        ];
        assert_eq!(
            group_ref_labels(&labels, &context(Some("feature"), &[]), false, true),
            vec![
                group(
                    "feature",
                    vec![
                        label("HEAD", RefLabelKind::Head),
                        label("feature", RefLabelKind::LocalBranch),
                    ]
                ),
                group("main", vec![label("main", RefLabelKind::LocalBranch)]),
                group("v1", vec![label("v1", RefLabelKind::Tag)]),
            ]
        );
    }

    #[test]
    fn head_alone_on_a_branch_forms_an_unnamed_group() {
        assert_eq!(
            group_ref_labels(
                &[label("HEAD", RefLabelKind::Head)],
                &context(Some("main"), &[]),
                false,
                false
            ),
            vec![group("", vec![label("HEAD", RefLabelKind::Head)])]
        );
    }

    #[test]
    fn detached_head_group_comes_first() {
        let labels = [
            label("v1", RefLabelKind::Tag),
            label("HEAD", RefLabelKind::Head),
            label("feature", RefLabelKind::LocalBranch),
        ];
        let groups = group_ref_labels(&labels, &context(None, &[]), false, false);
        assert_eq!(
            groups,
            vec![
                RefLabelGroup {
                    name: SharedString::new_static("Detached HEAD"),
                    labels: vec![label("HEAD", RefLabelKind::Head)],
                    is_detached_head: true,
                },
                group("feature", vec![label("feature", RefLabelKind::LocalBranch)]),
                group("", vec![label("v1", RefLabelKind::Tag)]),
            ]
        );
        assert_eq!(groups[0].icon_kinds(), Vec::<RefLabelKind>::new());
    }

    #[test]
    fn compact_mode_merges_everything_into_the_first_group() {
        let labels = [
            label("v1", RefLabelKind::Tag),
            label("feature", RefLabelKind::LocalBranch),
            label("origin/main", RefLabelKind::RemoteBranch),
            label("main", RefLabelKind::LocalBranch),
            label("HEAD", RefLabelKind::Head),
        ];
        assert_eq!(
            group_ref_labels(
                &labels,
                &context(Some("main"), &[("main", "origin/main")]),
                true,
                false
            ),
            vec![group(
                "origin & main",
                vec![
                    label("HEAD", RefLabelKind::Head),
                    label("main", RefLabelKind::LocalBranch),
                    label("origin/main", RefLabelKind::RemoteBranch),
                    label("feature", RefLabelKind::LocalBranch),
                    label("v1", RefLabelKind::Tag),
                ]
            )]
        );
    }

    #[test]
    fn compact_mode_keeps_current_and_tracked_groups_when_nothing_else_remains() {
        let labels = [
            label("origin/feature", RefLabelKind::RemoteBranch),
            label("feature", RefLabelKind::LocalBranch),
            label("main", RefLabelKind::LocalBranch),
        ];
        assert_eq!(
            group_ref_labels(
                &labels,
                &context(Some("main"), &[("feature", "origin/feature")]),
                true,
                false
            ),
            vec![
                group("main", vec![label("main", RefLabelKind::LocalBranch)]),
                group(
                    "origin & feature",
                    vec![
                        label("feature", RefLabelKind::LocalBranch),
                        label("origin/feature", RefLabelKind::RemoteBranch),
                    ]
                ),
            ]
        );
    }

    #[test]
    fn tag_group_names_are_hidden_unless_requested() {
        let labels = [
            label("v2", RefLabelKind::Tag),
            label("v1", RefLabelKind::Tag),
        ];
        let tags = vec![
            label("v1", RefLabelKind::Tag),
            label("v2", RefLabelKind::Tag),
        ];
        let no_branch = context(None, &[]);
        assert_eq!(
            group_ref_labels(&labels, &no_branch, false, false),
            vec![group("", tags.clone())]
        );
        assert_eq!(
            group_ref_labels(&labels, &no_branch, false, true),
            vec![group("v1", tags.clone())]
        );
        assert_eq!(
            group_ref_labels(&labels, &no_branch, true, false),
            vec![group("", tags.clone())]
        );
        assert_eq!(
            group_ref_labels(&labels, &no_branch, true, true),
            vec![group("v1", tags)]
        );
    }

    #[test]
    fn icon_kinds_are_doubled_for_repeated_kinds() {
        let tracked = group(
            "origin & main",
            vec![
                label("HEAD", RefLabelKind::Head),
                label("main", RefLabelKind::LocalBranch),
                label("origin/main", RefLabelKind::RemoteBranch),
            ],
        );
        assert_eq!(
            tracked.icon_kinds(),
            vec![
                RefLabelKind::Head,
                RefLabelKind::LocalBranch,
                RefLabelKind::RemoteBranch
            ]
        );
        let tags = group(
            "",
            vec![
                label("v1", RefLabelKind::Tag),
                label("v2", RefLabelKind::Tag),
                label("v3", RefLabelKind::Tag),
                label("refs/stash", RefLabelKind::Other),
            ],
        );
        assert_eq!(
            tags.icon_kinds(),
            vec![RefLabelKind::Tag, RefLabelKind::Tag, RefLabelKind::Other]
        );
    }

    #[test]
    fn collapsed_icon_colors_merge_equal_colors_across_groups() {
        let tag_color: Hsla = rgba(0x868A91FF).into();
        let branch_color: Hsla = rgba(0x5FAD65FF).into();
        assert_eq!(
            combined_colors(&[vec![branch_color], vec![tag_color], vec![tag_color]]),
            vec![branch_color, tag_color, tag_color]
        );
        assert_eq!(
            combined_colors(&[vec![branch_color], vec![tag_color]]),
            vec![branch_color, tag_color]
        );
    }

    #[test]
    fn shorten_ref_name_keeps_names_that_fit_or_are_short() {
        assert_eq!(
            shorten("feature/very-long-branch-name", 1000.),
            "feature/very-long-branch-name"
        );
        assert_eq!(shorten("short-name", 0.), "short-name");
    }

    #[test]
    fn shorten_ref_name_replaces_the_prefix_and_trims_to_the_available_width() {
        assert_eq!(
            shorten("feature/very-long-branch-name", 235.),
            "../very-long-branch-na\u{2026}"
        );
        assert_eq!(
            shorten("feature/very-long-branch-name", 150.),
            "../very-long-branch-n\u{2026}"
        );
        assert_eq!(
            shorten("feature/very-long-branch-name", 0.),
            "../very-long-branch-n\u{2026}"
        );
    }

    #[test]
    fn shorten_ref_name_keeps_a_short_prefix_before_the_separator() {
        assert_eq!(
            shorten("ab/cdefghijklmnopqrstuvwxyz", 0.),
            "ab/cdefghijklmnopqrst\u{2026}"
        );
        assert_eq!(
            shorten("abcdefghijklmnopqrstuvwxyz", -5.),
            "abcdefghijklmnopqrstu\u{2026}"
        );
    }

    #[test]
    fn label_icon_width_grows_by_two_units_per_extra_tag() {
        assert_eq!(label_icon_width(px(16.), 1), px(16.));
        assert_eq!(label_icon_width(px(16.), 2), px(21.));
        assert_eq!(label_icon_width(px(16.), 3), px(26.));
        assert_eq!(label_icon_width(px(15.), 2), px(20.));
        assert_eq!(label_icon_width(px(15.), 3), px(25.));
    }
}
