use std::{future::Future, ops::Range, time::Duration};

use collections::{HashMap, HashSet};
use futures::FutureExt as _;
use gpui::{App, AsyncWindowContext, Entity, EntityId, SharedString, Task};
use language::{
    Bias, Buffer, BufferSnapshot, HighlightId, LanguageAwareStyling, OutlineItem, Point,
};
use project::{DocumentHighlight, Location, Project, WorktreeId, lsp_store::OpenLspBufferHandle};

use crate::model::{RawUsage, UsageAccess};

const PARSE_WAIT_LIMIT: Duration = Duration::from_secs(1);
const LONG_LINE_LENGTH: usize = 4096;
const LONG_LINE_MARGIN: usize = 1024;
const IMPORT_NODE_KINDS: [&str; 4] = [
    "use_declaration",
    "using_directive",
    "preproc_include",
    "extern_crate_declaration",
];

pub(crate) struct UsageContext {
    pub(crate) origin: Option<(Entity<Buffer>, text::Anchor)>,
    pub(crate) current_buffer: Option<Entity<Buffer>>,
    pub(crate) open_buffers: HashSet<EntityId>,
    pub(crate) visible_worktrees: Vec<WorktreeId>,
}

pub(crate) struct AccessRequests {
    registrations: Vec<OpenLspBufferHandle>,
    requests: Vec<AccessRequest>,
}

struct AccessRequest {
    buffer: Entity<Buffer>,
    usages: Vec<(usize, Range<text::Anchor>)>,
    highlights: Task<anyhow::Result<Vec<DocumentHighlight>>>,
}

#[derive(Default)]
pub(crate) struct AccessOutcome {
    pub(crate) accesses: Vec<(usize, UsageAccess)>,
    pub(crate) error: Option<anyhow::Error>,
}

struct BufferFacts {
    snapshot: BufferSnapshot,
    worktree_order: usize,
    file_path: SharedString,
    file_name: SharedString,
    in_project: bool,
    in_open_file: bool,
    in_current_file: bool,
    origin_offset: Option<usize>,
}

struct LineFacts {
    text: SharedString,
    start: usize,
    end: usize,
    syntax_highlights: Vec<(Range<usize>, HighlightId)>,
    structure: Vec<SharedString>,
}

struct UsageSpan {
    location: Location,
    buffer_id: EntityId,
    start: usize,
    end: usize,
    row: u32,
    column: u32,
}

pub(crate) fn exclude_declarations(
    references: Vec<Location>,
    declarations: &[Location],
    cx: &App,
) -> Vec<Location> {
    let declaration_ranges = declarations
        .iter()
        .filter_map(|declaration| {
            let buffer = declaration.buffer.read(cx);
            let start = buffer.summary_for_anchor::<Point>(&declaration.range.start);
            let end = buffer.summary_for_anchor::<Point>(&declaration.range.end);
            (start.row == end.row).then(|| {
                let start = buffer.point_to_offset(start);
                let end = buffer.point_to_offset(end);
                (
                    declaration.buffer.entity_id(),
                    start.min(end)..start.max(end),
                )
            })
        })
        .collect::<Vec<_>>();
    if declaration_ranges.is_empty() {
        return references;
    }
    references
        .into_iter()
        .filter(|reference| {
            let buffer_id = reference.buffer.entity_id();
            let buffer = reference.buffer.read(cx);
            let first = buffer.summary_for_anchor::<usize>(&reference.range.start);
            let second = buffer.summary_for_anchor::<usize>(&reference.range.end);
            let start = first.min(second);
            let end = first.max(second);
            !declaration_ranges
                .iter()
                .any(|(declaration_buffer, declaration_range)| {
                    *declaration_buffer == buffer_id
                        && declaration_range.start <= start
                        && end <= declaration_range.end
                })
        })
        .collect()
}

pub(crate) fn parsing_settled(
    locations: &[Location],
    cx: &App,
) -> impl Future<Output = ()> + use<> {
    let mut seen_buffers = HashSet::default();
    let parsing = locations
        .iter()
        .filter(|location| seen_buffers.insert(location.buffer.entity_id()))
        .map(|location| location.buffer.read(cx).parsing_idle())
        .collect::<Vec<_>>();
    let timeout = cx.background_executor().timer(PARSE_WAIT_LIMIT);
    async move {
        futures::select_biased! {
            _ = futures::future::join_all(parsing).fuse() => {}
            () = timeout.fuse() => {}
        }
    }
}

pub(crate) fn raw_usages(
    locations: Vec<Location>,
    context: &UsageContext,
    project: &Project,
    cx: &App,
) -> Vec<RawUsage<Location>> {
    let mut buffers: HashMap<EntityId, Option<BufferFacts>> = HashMap::default();
    let mut spans = Vec::with_capacity(locations.len());
    for location in locations {
        let buffer_id = location.buffer.entity_id();
        let facts = buffers
            .entry(buffer_id)
            .or_insert_with(|| buffer_facts(&location.buffer, context, project, cx));
        let Some(facts) = facts.as_ref() else {
            continue;
        };
        let snapshot = &facts.snapshot;
        let first = snapshot.summary_for_anchor::<usize>(&location.range.start);
        let second = snapshot.summary_for_anchor::<usize>(&location.range.end);
        let start = first.min(second);
        let end = first.max(second);
        let row = snapshot.offset_to_point(start).row;
        spans.push(UsageSpan {
            location,
            buffer_id,
            start,
            end,
            row,
            column: 0,
        });
    }
    spans.sort_by_key(|span| (span.buffer_id.as_u64(), span.start, span.end));

    let mut lines: HashMap<(EntityId, u32), LineFacts> = HashMap::default();
    let mut previous_span: Option<(EntityId, u32, usize, u32)> = None;
    for span in &mut spans {
        let Some(Some(facts)) = buffers.get(&span.buffer_id) else {
            continue;
        };
        let snapshot = &facts.snapshot;
        let (count_from, base_column) = match previous_span {
            Some((buffer_id, row, offset, column))
                if buffer_id == span.buffer_id && row == span.row =>
            {
                (offset, column)
            }
            _ => (snapshot.point_to_offset(Point::new(span.row, 0)), 0),
        };
        let counted = snapshot
            .text_for_range(count_from..span.start)
            .map(|chunk| chunk.chars().count())
            .sum::<usize>();
        span.column = base_column.saturating_add(u32::try_from(counted).unwrap_or(u32::MAX));
        previous_span = Some((span.buffer_id, span.row, span.start, span.column));
        let first_usage = span.start..span.end;
        lines
            .entry((span.buffer_id, span.row))
            .or_insert_with(|| line_facts(snapshot, span.row, first_usage));
    }

    spans
        .into_iter()
        .filter_map(|span| {
            let facts = buffers.get(&span.buffer_id)?.as_ref()?;
            let line = lines.get(&(span.buffer_id, span.row))?;
            let (is_import, is_comment) = syntax_flags(&facts.snapshot, span.start..span.end);
            let match_start = span
                .start
                .max(line.start)
                .min(line.end)
                .saturating_sub(line.start);
            let match_end = span
                .end
                .max(line.start)
                .min(line.end)
                .saturating_sub(line.start);
            Some(RawUsage {
                location: span.location,
                worktree_order: facts.worktree_order,
                file_path: facts.file_path.clone(),
                file_name: facts.file_name.clone(),
                row: span.row,
                column: span.column,
                offset: span.start,
                line_text: line.text.clone(),
                match_range: match_start.min(match_end)..match_start.max(match_end),
                syntax_highlights: line.syntax_highlights.clone(),
                structure: line.structure.clone(),
                access: UsageAccess::Unknown,
                is_import,
                is_comment,
                is_origin: facts
                    .origin_offset
                    .is_some_and(|caret| span.start <= caret && caret <= span.end),
                in_project: facts.in_project,
                in_open_file: facts.in_open_file,
                in_current_file: facts.in_current_file,
            })
        })
        .collect()
}

pub(crate) fn request_access(
    project: &Entity<Project>,
    usages: &[RawUsage<Location>],
    cx: &mut App,
) -> AccessRequests {
    let mut groups: Vec<(Entity<Buffer>, Vec<(usize, Range<text::Anchor>)>)> = Vec::new();
    let mut group_of_buffer: HashMap<EntityId, usize> = HashMap::default();
    for (raw_index, usage) in usages.iter().enumerate() {
        let group_index = *group_of_buffer
            .entry(usage.location.buffer.entity_id())
            .or_insert_with(|| {
                groups.push((usage.location.buffer.clone(), Vec::new()));
                groups.len() - 1
            });
        if let Some((_, members)) = groups.get_mut(group_index) {
            members.push((raw_index, usage.location.range.clone()));
        }
    }

    let mut registrations = Vec::with_capacity(groups.len());
    let mut requests = Vec::with_capacity(groups.len());
    for (buffer, members) in groups {
        let Some(position) = members.first().map(|(_, range)| range.start) else {
            continue;
        };
        let (registration, highlights) = project.update(cx, |project, cx| {
            let registration = project.register_buffer_with_language_servers(&buffer, cx);
            let highlights = project.document_highlights(&buffer, position, cx);
            (registration, highlights)
        });
        registrations.push(registration);
        requests.push(AccessRequest {
            buffer,
            usages: members,
            highlights,
        });
    }
    AccessRequests {
        registrations,
        requests,
    }
}

pub(crate) async fn resolve_access(
    requests: AccessRequests,
    cx: &mut AsyncWindowContext,
) -> AccessOutcome {
    let AccessRequests {
        registrations,
        requests,
    } = requests;
    let mut outcome = AccessOutcome::default();
    for request in requests {
        match request.highlights.await {
            Ok(highlights) => {
                let snapshot = request.buffer.read_with(cx, |buffer, _| buffer.snapshot());
                let highlight_ranges = highlights
                    .iter()
                    .map(|highlight| {
                        (
                            ordered_range(&snapshot, &highlight.range),
                            access_for_kind(highlight.kind),
                        )
                    })
                    .collect::<Vec<_>>();
                for (raw_index, usage_range) in request.usages {
                    let usage = ordered_range(&snapshot, &usage_range);
                    let access = highlight_ranges
                        .iter()
                        .find(|(highlight, _)| {
                            highlight.start <= usage.start && usage.end <= highlight.end
                        })
                        .map(|(_, access)| *access)
                        .unwrap_or_default();
                    if access != UsageAccess::Unknown {
                        outcome.accesses.push((raw_index, access));
                    }
                }
            }
            Err(error) => {
                log::error!("Show Usages read/write classification failed: {error:#}");
                if outcome.error.is_none() {
                    outcome.error = Some(error);
                }
            }
        }
    }
    drop(registrations);
    outcome
}

fn ordered_range(snapshot: &BufferSnapshot, range: &Range<text::Anchor>) -> Range<usize> {
    let first = snapshot.summary_for_anchor::<usize>(&range.start);
    let second = snapshot.summary_for_anchor::<usize>(&range.end);
    first.min(second)..first.max(second)
}

fn access_for_kind(kind: lsp::DocumentHighlightKind) -> UsageAccess {
    if kind == lsp::DocumentHighlightKind::WRITE {
        UsageAccess::Write
    } else if kind == lsp::DocumentHighlightKind::READ {
        UsageAccess::Read
    } else {
        UsageAccess::Unknown
    }
}

fn buffer_facts(
    buffer: &Entity<Buffer>,
    context: &UsageContext,
    project: &Project,
    cx: &App,
) -> Option<BufferFacts> {
    let buffer_state = buffer.read(cx);
    let file = buffer_state.file()?;
    let worktree_id = file.worktree_id(cx);
    let worktree = project.worktree_for_id(worktree_id, cx);
    let is_visible = worktree
        .as_ref()
        .is_some_and(|worktree| worktree.read(cx).is_visible());
    let relative_path = file.path();
    let in_project = is_visible
        && worktree
            .as_ref()
            .and_then(|worktree| {
                worktree
                    .read(cx)
                    .entry_for_path(relative_path)
                    .map(|entry| !entry.is_ignored && !entry.is_external)
            })
            .unwrap_or(false);
    let file_path = if is_visible && !relative_path.is_empty() {
        SharedString::from(relative_path.as_unix_str())
    } else {
        let absolute_path = match file.as_local() {
            Some(local_file) => local_file.abs_path(cx),
            None => file.full_path(cx),
        };
        SharedString::from(absolute_path.to_string_lossy())
    };
    let snapshot = buffer_state.snapshot();
    let origin_offset = context
        .origin
        .as_ref()
        .filter(|(origin_buffer, _)| origin_buffer == buffer)
        .map(|(_, position)| snapshot.summary_for_anchor::<usize>(position));
    Some(BufferFacts {
        worktree_order: context
            .visible_worktrees
            .iter()
            .position(|visible_worktree| *visible_worktree == worktree_id)
            .unwrap_or(usize::MAX),
        file_path,
        file_name: SharedString::from(file.file_name(cx)),
        in_project,
        in_open_file: context.open_buffers.contains(&buffer.entity_id()),
        in_current_file: context.current_buffer.as_ref() == Some(buffer),
        origin_offset,
        snapshot,
    })
}

fn line_facts(snapshot: &BufferSnapshot, row: u32, first_usage: Range<usize>) -> LineFacts {
    let line_start = snapshot.point_to_offset(Point::new(row, 0));
    let line_end = snapshot.point_to_offset(Point::new(row, snapshot.line_len(row)));
    let (start, end) = if line_end.saturating_sub(line_start) > LONG_LINE_LENGTH {
        let window_start = first_usage
            .start
            .saturating_sub(LONG_LINE_MARGIN)
            .max(line_start);
        let window_end = first_usage
            .start
            .saturating_add(LONG_LINE_MARGIN)
            .min(line_end);
        (
            snapshot.clip_offset(window_start, Bias::Left),
            snapshot.clip_offset(window_end, Bias::Right),
        )
    } else {
        (line_start, line_end)
    };

    let text = snapshot.text_for_range(start..end).collect::<String>();
    let mut syntax_highlights = Vec::new();
    let mut offset = 0;
    for chunk in snapshot.chunks(
        start..end,
        LanguageAwareStyling {
            tree_sitter: true,
            diagnostics: false,
        },
    ) {
        let chunk_length = chunk.text.len();
        if let Some(highlight_id) = chunk.syntax_highlight_id {
            syntax_highlights.push((offset..offset + chunk_length, highlight_id));
        }
        offset += chunk_length;
    }

    LineFacts {
        text: SharedString::from(text),
        start,
        end,
        syntax_highlights,
        structure: enclosing_structure(snapshot, first_usage),
    }
}

fn enclosing_structure(snapshot: &BufferSnapshot, usage: Range<usize>) -> Vec<SharedString> {
    snapshot
        .outline_items_as_offsets_containing(usage.clone(), false, None)
        .iter()
        .filter(|item| item.range.start <= usage.start && usage.end <= item.range.end)
        .filter(|item| {
            !(item.selection_range.start <= usage.start && usage.end <= item.selection_range.end)
        })
        .filter_map(outline_item_name)
        .collect()
}

fn outline_item_name(item: &OutlineItem<usize>) -> Option<SharedString> {
    let text = item.text.as_str();
    let name = match (item.name_ranges.first(), item.name_ranges.last()) {
        (Some(first), Some(last)) => text.get(first.start..last.end).unwrap_or(text),
        _ => text,
    };
    let name = name.trim();
    (!name.is_empty()).then(|| SharedString::from(name))
}

fn syntax_flags(snapshot: &BufferSnapshot, usage: Range<usize>) -> (bool, bool) {
    let mut is_import = false;
    let mut is_comment = false;
    let mut node = snapshot.syntax_ancestor(usage);
    while let Some(current) = node {
        let kind = current.kind();
        is_comment |= kind.contains("comment");
        is_import |= kind.contains("import") || IMPORT_NODE_KINDS.contains(&kind);
        node = current.parent();
    }
    (is_import, is_comment)
}
