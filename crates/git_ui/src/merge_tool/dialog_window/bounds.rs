use db::kvp::{KeyValueStore, ScopedKeyValueStore};
use gpui::{App, Bounds, DisplayId, Pixels, Size, Window};
use util::ResultExt as _;

const BOUNDS_NAMESPACE: &str = "merge_tool_dialog_windows";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FrameRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl FrameRect {
    pub(super) fn from_bounds(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: bounds.origin.x.as_f32(),
            y: bounds.origin.y.as_f32(),
            width: bounds.size.width.as_f32(),
            height: bounds.size.height.as_f32(),
        }
    }

    pub(super) fn size(self) -> FrameSize {
        FrameSize {
            width: self.width,
            height: self.height,
        }
    }

    fn right(self) -> f32 {
        self.x + self.width
    }

    fn bottom(self) -> f32 {
        self.y + self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FrameSize {
    pub width: f32,
    pub height: f32,
}

impl FrameSize {
    pub(super) fn from_size(size: Size<Pixels>) -> Self {
        Self {
            width: size.width.as_f32(),
            height: size.height.as_f32(),
        }
    }

    fn from_saved(saved: SavedSize) -> Self {
        Self {
            width: saved.width as f32,
            height: saved.height as f32,
        }
    }

    fn at_least(self, minimum: FrameSize) -> Self {
        Self {
            width: self.width.max(minimum.width),
            height: self.height.max(minimum.height),
        }
    }

    fn rounded(self) -> SavedSize {
        SavedSize {
            width: self.width.round().max(0.) as u32,
            height: self.height.round().max(0.) as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SavedLocation {
    pub x: i32,
    pub y: i32,
    pub display: Option<String>,
}

impl SavedLocation {
    pub(super) fn serialize(&self) -> String {
        match &self.display {
            Some(display) => format!("{} {} {}", self.x, self.y, display),
            None => format!("{} {}", self.x, self.y),
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split_whitespace();
        let x = parts.next()?.parse().ok()?;
        let y = parts.next()?.parse().ok()?;
        let display = parts.next().map(str::to_owned);
        if parts.next().is_some() {
            return None;
        }
        Some(Self { x, y, display })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SavedSize {
    pub width: u32,
    pub height: u32,
}

impl SavedSize {
    pub(super) fn serialize(self) -> String {
        format!("{} {}", self.width, self.height)
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split_whitespace();
        let width: u32 = parts.next()?.parse().ok()?;
        let height: u32 = parts.next()?.parse().ok()?;
        if parts.next().is_some() || width == 0 || height == 0 {
            return None;
        }
        Some(Self { width, height })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct SavedBounds {
    pub location: Option<SavedLocation>,
    pub size: Option<SavedSize>,
}

pub(super) struct ScreenGeometry {
    pub id: DisplayId,
    pub uuid: Option<String>,
    pub visible: FrameRect,
}

pub(super) struct ScreenSet {
    pub screens: Vec<ScreenGeometry>,
    pub primary: Option<usize>,
}

impl ScreenSet {
    pub(super) fn capture(cx: &App) -> Self {
        let screens: Vec<ScreenGeometry> = cx
            .displays()
            .iter()
            .map(|display| ScreenGeometry {
                id: display.id(),
                uuid: display.uuid().log_err().map(|uuid| uuid.to_string()),
                visible: FrameRect::from_bounds(display.visible_bounds()),
            })
            .collect();
        let primary = cx.primary_display().and_then(|display| {
            let primary_id = display.id();
            screens.iter().position(|screen| screen.id == primary_id)
        });
        Self { screens, primary }
    }

    fn index_of_uuid(&self, uuid: &str) -> Option<usize> {
        self.screens
            .iter()
            .position(|screen| screen.uuid.as_deref() == Some(uuid))
    }

    fn index_of_id(&self, id: DisplayId) -> Option<usize> {
        self.screens.iter().position(|screen| screen.id == id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnerPlacement {
    pub frame: FrameRect,
    pub display: Option<DisplayId>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlannedBounds {
    pub frame: FrameRect,
    pub display: Option<DisplayId>,
    pub size_reference: SavedSize,
}

pub(super) fn plan_bounds(
    initial: FrameSize,
    minimum: FrameSize,
    saved: &SavedBounds,
    owner: &OwnerPlacement,
    screens: &ScreenSet,
) -> PlannedBounds {
    let window_size = saved
        .size
        .map(FrameSize::from_saved)
        .unwrap_or(initial)
        .at_least(minimum);
    let screen_index = choose_screen(saved, owner, screens);
    let screen = screen_index.and_then(|index| screens.screens.get(index));
    let (x, y) = match &saved.location {
        Some(location) => (location.x as f32, location.y as f32),
        None => centered_origin(owner.frame, window_size),
    };
    let mut frame = FrameRect {
        x,
        y,
        width: window_size.width,
        height: window_size.height,
    };
    if let Some(screen) = screen {
        frame = fit_to_screen(frame, screen.visible);
    }
    PlannedBounds {
        frame,
        display: screen.map(|screen| screen.id),
        size_reference: saved.size.unwrap_or_else(|| window_size.rounded()),
    }
}

fn choose_screen(
    saved: &SavedBounds,
    owner: &OwnerPlacement,
    screens: &ScreenSet,
) -> Option<usize> {
    saved
        .location
        .as_ref()
        .and_then(|location| location.display.as_deref())
        .and_then(|uuid| screens.index_of_uuid(uuid))
        .or_else(|| owner.display.and_then(|id| screens.index_of_id(id)))
        .or(screens.primary)
        .or_else(|| (!screens.screens.is_empty()).then_some(0))
}

fn centered_origin(container: FrameRect, window_size: FrameSize) -> (f32, f32) {
    (
        container.x + ((container.width - window_size.width) / 2.).trunc(),
        container.y + ((container.height - window_size.height) / 2.).trunc(),
    )
}

pub(super) fn fit_to_screen(mut frame: FrameRect, screen: FrameRect) -> FrameRect {
    if frame.right() > screen.right() {
        frame.x = screen.right() - frame.width;
    }
    if frame.x < screen.x {
        frame.x = screen.x;
    }
    if frame.right() > screen.right() {
        frame.width = screen.right() - frame.x;
    }
    if frame.bottom() > screen.bottom() {
        frame.y = screen.bottom() - frame.height;
    }
    if frame.y < screen.y {
        frame.y = screen.y;
    }
    if frame.bottom() > screen.bottom() {
        frame.height = screen.bottom() - frame.y;
    }
    frame
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct BoundsToSave {
    pub location: SavedLocation,
    pub size: Option<SavedSize>,
}

pub(super) fn bounds_to_save(
    frame: FrameRect,
    display_uuid: Option<String>,
    size_reference: SavedSize,
) -> BoundsToSave {
    let current_size = frame.size().rounded();
    BoundsToSave {
        location: SavedLocation {
            x: frame.x.round() as i32,
            y: frame.y.round() as i32,
            display: display_uuid,
        },
        size: (current_size != size_reference).then_some(current_size),
    }
}

fn storage_key(bounds_key: &str, project_scope: Option<&str>) -> String {
    match project_scope {
        Some(project_scope) => format!("{project_scope}:{bounds_key}"),
        None => bounds_key.to_owned(),
    }
}

fn location_key(storage_key: &str) -> String {
    format!("{storage_key}:location")
}

fn size_key(storage_key: &str) -> String {
    format!("{storage_key}:size")
}

fn read_entry<T>(
    store: &ScopedKeyValueStore<'_>,
    key: &str,
    parse: fn(&str) -> Option<T>,
) -> Option<T> {
    let value = store.read(key).log_err().flatten()?;
    let parsed = parse(&value);
    if parsed.is_none() {
        log::warn!("ignoring malformed dialog window bounds {value:?} stored under {key}");
    }
    parsed
}

pub(super) fn load_saved_bounds(
    bounds_key: &str,
    project_scope: Option<&str>,
    cx: &App,
) -> SavedBounds {
    let key_value_store = KeyValueStore::global(cx);
    let store = key_value_store.scoped(BOUNDS_NAMESPACE);
    let storage_key = storage_key(bounds_key, project_scope);
    SavedBounds {
        location: read_entry(&store, &location_key(&storage_key), SavedLocation::parse),
        size: read_entry(&store, &size_key(&storage_key), SavedSize::parse),
    }
}

pub(super) fn save_bounds(
    bounds_key: &str,
    project_scope: Option<&str>,
    bounds: BoundsToSave,
    cx: &App,
) {
    let key_value_store = KeyValueStore::global(cx);
    let storage_key = storage_key(bounds_key, project_scope);
    let location_key = location_key(&storage_key);
    let size_key = size_key(&storage_key);
    db::write_and_log(cx, move || async move {
        let store = key_value_store.scoped(BOUNDS_NAMESPACE);
        store
            .write(location_key, bounds.location.serialize())
            .await?;
        if let Some(size) = bounds.size {
            store.write(size_key, size.serialize()).await?;
        }
        anyhow::Ok(())
    });
}

pub(super) struct TrackedBounds {
    bounds_key: &'static str,
    project_scope: Option<String>,
    size_reference: SavedSize,
    frame: FrameRect,
    display: Option<DisplayId>,
}

impl TrackedBounds {
    pub(super) fn new(
        bounds_key: &'static str,
        project_scope: Option<String>,
        planned: &PlannedBounds,
    ) -> Self {
        Self {
            bounds_key,
            project_scope,
            size_reference: planned.size_reference,
            frame: planned.frame,
            display: planned.display,
        }
    }

    pub(super) fn project_scope(&self) -> Option<&str> {
        self.project_scope.as_deref()
    }

    pub(super) fn remember(&mut self, window: &Window, cx: &App) {
        let Some(display) = window.display(cx) else {
            return;
        };
        self.frame = FrameRect::from_bounds(window.bounds());
        self.display = Some(display.id());
    }

    pub(super) fn save(&self, cx: &App) {
        let display_uuid = self
            .display
            .and_then(|id| cx.find_display(id))
            .and_then(|display| display.uuid().log_err())
            .map(|uuid| uuid.to_string());
        save_bounds(
            self.bounds_key,
            self.project_scope(),
            bounds_to_save(self.frame, display_uuid, self.size_reference),
            cx,
        );
    }
}
