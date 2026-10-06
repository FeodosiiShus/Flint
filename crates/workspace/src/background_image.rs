use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Weak},
};

use anyhow::{Context as _, Result, bail};
use collections::{HashMap, HashSet};
use fs::{Fs, MTime};
use gpui::{App, AppContext as _, Context, PathPromptOptions, RenderImage, Window, WindowId};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use project::DirectoryLister;
use settings::{BackgroundImageAnchor, BackgroundImageFill, Settings as _, SettingsLocation};
use ui::{BackgroundImageLayers, BackgroundImagePaintLayer, BackgroundImageTarget};
use util::{paths::PathExt as _, rel_path::RelPath};

use crate::{
    BackgroundImageLayerSettings, BackgroundImageSettings, Workspace,
    notifications::{
        NotificationId, NotifyTaskExt as _, simple_message_notification::MessageNotification,
    },
};

gpui::actions!(
    workspace,
    [
        SelectBackgroundImage,
        SelectEmptyFrameBackgroundImage,
        ClearBackgroundImage,
    ]
);

const MAX_BACKGROUND_IMAGE_SIDE: u32 = 8192;
const MAX_DISPLAY_BACKING_SCALE_FACTOR: f32 = 2.0;
const MIN_TILED_BACKGROUND_IMAGE_SIDE: u32 = 256;
const BACKGROUND_IMAGE_TARGETS: [BackgroundImageTarget; 2] = [
    BackgroundImageTarget::EditorAndTools,
    BackgroundImageTarget::EmptyFrame,
];
const SUPPORTED_BACKGROUND_IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "tga", "qoi", "pnm", "pbm",
    "pgm", "ppm", "hdr", "exr", "dds", "ff",
];

type CoverTarget = (u32, u32);

static DECODED_BACKGROUND_IMAGES: LazyLock<
    futures::lock::Mutex<HashMap<BackgroundImageKey, Weak<RenderImage>>>,
> = LazyLock::new(|| futures::lock::Mutex::new(HashMap::default()));

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BackgroundImageKey {
    path: PathBuf,
    mtime: MTime,
    opacity: u32,
    flip_horizontal: bool,
    flip_vertical: bool,
    tile_expanded: bool,
    cover_target: Option<CoverTarget>,
}

impl BackgroundImageKey {
    fn new(
        layer: &BackgroundImageLayerSettings,
        mtime: MTime,
        cover_target: Option<CoverTarget>,
    ) -> Self {
        Self {
            path: layer.path.clone(),
            mtime,
            opacity: layer.opacity,
            flip_horizontal: layer.flip_horizontal,
            flip_vertical: layer.flip_vertical,
            tile_expanded: layer.fill == BackgroundImageFill::Tile,
            cover_target,
        }
    }
}

struct LoadedBackgroundImage {
    key: BackgroundImageKey,
    image: Arc<RenderImage>,
}

enum BackgroundImageLoad {
    Unchanged,
    Loaded(LoadedBackgroundImage),
}

#[derive(Default)]
struct BackgroundImageTargetState {
    layer: Option<BackgroundImageLayerSettings>,
    loaded: Option<LoadedBackgroundImage>,
    _load_task: Option<gpui::Task<()>>,
    notified_errors: HashSet<(PathBuf, String)>,
}

impl BackgroundImageTargetState {
    fn paint_layer(&self) -> Option<BackgroundImagePaintLayer> {
        let layer = self.layer.as_ref()?;
        let loaded = self.loaded.as_ref()?;
        Some(BackgroundImagePaintLayer {
            image: loaded.image.clone(),
            mode: background_image_mode(layer.fill),
            alignment: background_image_alignment(layer.anchor),
        })
    }
}

#[derive(Default)]
pub(crate) struct BackgroundImageState {
    editor_and_tools: BackgroundImageTargetState,
    empty_frame: BackgroundImageTargetState,
    settings: Option<BackgroundImageSettings>,
    published_window: Option<WindowId>,
}

impl BackgroundImageState {
    fn target_mut(&mut self, target: BackgroundImageTarget) -> &mut BackgroundImageTargetState {
        match target {
            BackgroundImageTarget::EditorAndTools => &mut self.editor_and_tools,
            BackgroundImageTarget::EmptyFrame => &mut self.empty_frame,
        }
    }

    fn window_images(&self) -> ui::WindowBackgroundImages {
        ui::WindowBackgroundImages {
            editor_and_tools: self.editor_and_tools.paint_layer(),
            empty_frame: self.empty_frame.paint_layer(),
        }
    }

    fn take_images(&mut self) -> Vec<Arc<RenderImage>> {
        [&mut self.editor_and_tools, &mut self.empty_frame]
            .into_iter()
            .filter_map(|state| {
                state._load_task = None;
                state.loaded.take()
            })
            .map(|loaded| loaded.image)
            .collect()
    }
}

impl Workspace {
    pub(crate) fn refresh_background_images(
        &mut self,
        reload: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.resolve_background_image_settings(cx);
        if !reload && self.background_image.settings.as_ref() == Some(&settings) {
            return;
        }
        let mut retired_images = Vec::new();
        for target in BACKGROUND_IMAGE_TARGETS {
            let layer = background_image_layer_settings(&settings, target).cloned();
            let retired_image = self.refresh_background_image_target(target, layer, window, cx);
            retired_images.extend(retired_image);
        }
        self.background_image.settings = Some(settings);
        self.publish_background_images(window, cx);
        for image in retired_images {
            drop_unshared_background_image(image, Some(&mut *window), cx);
        }
    }

    pub(crate) fn publish_background_images(&mut self, window: &mut Window, cx: &mut App) {
        if !self.owns_window_chrome() {
            return;
        }
        let window_id = window.window_handle().window_id();
        self.background_image.published_window = Some(window_id);
        let images = self.background_image.window_images();
        let published = BackgroundImageLayers::window(window_id, cx)
            .cloned()
            .unwrap_or_default();
        if published == images {
            return;
        }
        BackgroundImageLayers::set_window(window_id, images, cx);
        window.refresh();
    }

    pub(crate) fn release_background_images(&mut self, cx: &mut App) {
        let owns_window_chrome = self.owns_window_chrome();
        let images = self.background_image.take_images();
        if let Some(window_id) = self.background_image.published_window.take() {
            let holds_own_images = BackgroundImageLayers::window(window_id, cx)
                .is_some_and(|published| holds_any_image(published, &images));
            if owns_window_chrome && holds_own_images {
                BackgroundImageLayers::remove_window(window_id, cx);
            }
        }
        for image in images {
            drop_unshared_background_image(image, None, cx);
        }
    }

    pub(crate) fn select_background_image(
        &mut self,
        _: &SelectBackgroundImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_for_background_image(BackgroundImageTarget::EditorAndTools, window, cx);
    }

    pub(crate) fn select_empty_frame_background_image(
        &mut self,
        _: &SelectEmptyFrameBackgroundImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_for_background_image(BackgroundImageTarget::EmptyFrame, window, cx);
    }

    pub(crate) fn clear_background_image(
        &mut self,
        _: &ClearBackgroundImage,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        settings::update_settings_file(self.app_state.fs.clone(), cx, |settings, _| {
            let background_image = settings.project.background_image.get_or_insert_default();
            let editor_and_tools = background_image.editor_and_tools.get_or_insert_default();
            editor_and_tools.path = Some(String::new());
            background_image.empty_frame.get_or_insert_default().path = Some(String::new());
        });
    }

    fn prompt_for_background_image(
        &mut self,
        target: BackgroundImageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fs = self.app_state.fs.clone();
        let paths = self.prompt_for_open_path(
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Select Background Image".into()),
            },
            DirectoryLister::Local(self.project.clone(), fs.clone()),
            window,
            cx,
        );
        cx.spawn_in(window, async move |_, cx| {
            let Some(path) = paths.await?.and_then(|paths| paths.into_iter().next()) else {
                return anyhow::Ok(());
            };
            if !is_supported_background_image(&path) {
                bail!("Unsupported background image format: {}", path.display());
            }
            let compacted_path = path.compact().to_string_lossy().into_owned();
            cx.update(|_, cx| {
                settings::update_settings_file(fs, cx, move |settings, _| {
                    let background_image =
                        settings.project.background_image.get_or_insert_default();
                    let layer = match target {
                        BackgroundImageTarget::EditorAndTools => {
                            background_image.editor_and_tools.get_or_insert_default()
                        }
                        BackgroundImageTarget::EmptyFrame => {
                            background_image.empty_frame.get_or_insert_default()
                        }
                    };
                    layer.path = Some(compacted_path);
                });
            })?;
            anyhow::Ok(())
        })
        .detach_and_notify_err(self.weak_self.clone(), window, cx);
    }

    fn resolve_background_image_settings(&self, cx: &App) -> BackgroundImageSettings {
        let worktree_id = self
            .project
            .read(cx)
            .visible_worktrees(cx)
            .next()
            .map(|worktree| worktree.read(cx).id());
        let location = worktree_id.map(|worktree_id| SettingsLocation {
            worktree_id,
            path: RelPath::empty(),
        });
        BackgroundImageSettings::get(location, cx).clone()
    }

    fn refresh_background_image_target(
        &mut self,
        target: BackgroundImageTarget,
        layer: Option<BackgroundImageLayerSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        let fs = self.app_state.fs.clone();
        let state = self.background_image.target_mut(target);
        state.layer = layer.clone();
        let Some(layer) = layer.filter(|layer| layer.opacity > 0) else {
            state._load_task = None;
            return state.loaded.take().map(|loaded| loaded.image);
        };
        let cached_key = state.loaded.as_ref().map(|loaded| loaded.key.clone());
        let cover_target = if layer.fill == BackgroundImageFill::Scale {
            scale_cover_target(window, cx)
        } else {
            None
        };
        let load = cx.background_spawn(load_background_image(
            fs,
            layer.clone(),
            cached_key,
            cover_target,
        ));
        state._load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = load.await;
            this.update_in(cx, |this, window, cx| {
                this.finish_background_image_load(target, layer, result, window, cx);
            })
            .ok();
        }));
        None
    }

    fn finish_background_image_load(
        &mut self,
        target: BackgroundImageTarget,
        layer: BackgroundImageLayerSettings,
        result: Result<BackgroundImageLoad>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.background_image.target_mut(target);
        state._load_task = None;
        let (retired_image, error_message) = match result {
            Ok(BackgroundImageLoad::Unchanged) => (None, None),
            Ok(BackgroundImageLoad::Loaded(loaded)) => (state.loaded.replace(loaded), None),
            Err(error) => {
                let message = format!("{error:#}");
                log::error!(
                    "Failed to load background image {}: {message}",
                    layer.path.display()
                );
                let is_new_error = state.notified_errors.insert((layer.path, message.clone()));
                (state.loaded.take(), is_new_error.then_some(message))
            }
        };
        self.publish_background_images(window, cx);
        if let Some(retired_image) = retired_image {
            drop_unshared_background_image(retired_image.image, Some(window), cx);
        }
        if let Some(message) = error_message {
            self.show_notification(background_image_notification_id(target), cx, |cx| {
                cx.new(|cx| MessageNotification::from_workspace_error(message, cx))
            });
        }
    }
}

fn background_image_layer_settings(
    settings: &BackgroundImageSettings,
    target: BackgroundImageTarget,
) -> Option<&BackgroundImageLayerSettings> {
    match target {
        BackgroundImageTarget::EditorAndTools => settings.editor_and_tools.as_ref(),
        BackgroundImageTarget::EmptyFrame => settings.empty_frame.as_ref(),
    }
}

fn background_image_notification_id(target: BackgroundImageTarget) -> NotificationId {
    match target {
        BackgroundImageTarget::EditorAndTools => {
            NotificationId::composite::<BackgroundImageState>("editor-and-tools")
        }
        BackgroundImageTarget::EmptyFrame => {
            NotificationId::composite::<BackgroundImageState>("empty-frame")
        }
    }
}

fn holds_any_image(published: &ui::WindowBackgroundImages, images: &[Arc<RenderImage>]) -> bool {
    BACKGROUND_IMAGE_TARGETS
        .into_iter()
        .filter_map(|target| published.layer(target))
        .any(|layer| images.iter().any(|image| Arc::ptr_eq(image, &layer.image)))
}

fn drop_unshared_background_image(
    image: Arc<RenderImage>,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    if Arc::strong_count(&image) == 1 {
        cx.drop_image(image, window);
    }
}

fn scale_cover_target(window: &Window, cx: &App) -> Option<CoverTarget> {
    let viewport_size = window.viewport_size();
    let window_scale_factor = window.scale_factor();
    let mut width = f32::from(viewport_size.width) * window_scale_factor;
    let mut height = f32::from(viewport_size.height) * window_scale_factor;
    for display in cx.displays() {
        let display_size = display.bounds().size;
        width = width.max(f32::from(display_size.width) * MAX_DISPLAY_BACKING_SCALE_FACTOR);
        height = height.max(f32::from(display_size.height) * MAX_DISPLAY_BACKING_SCALE_FACTOR);
    }
    Some((pixel_count(width)?, pixel_count(height)?))
}

fn pixel_count(value: f32) -> Option<u32> {
    if !value.is_finite() || value < 1.0 {
        return None;
    }
    Some(value.ceil() as u32)
}

fn cover_dimensions(source: (u32, u32), target: CoverTarget) -> Option<(u32, u32)> {
    let (source_width, source_height) = source;
    let (target_width, target_height) = target;
    if source_width == 0 || source_height == 0 || target_width == 0 || target_height == 0 {
        return None;
    }
    let width_driven = u64::from(target_width) * u64::from(source_height)
        >= u64::from(target_height) * u64::from(source_width);
    let (width, height) = if width_driven {
        if target_width >= source_width {
            return None;
        }
        let height =
            (u64::from(source_height) * u64::from(target_width)).div_ceil(u64::from(source_width));
        (u64::from(target_width), height)
    } else {
        if target_height >= source_height {
            return None;
        }
        let width =
            (u64::from(source_width) * u64::from(target_height)).div_ceil(u64::from(source_height));
        (width, u64::from(target_height))
    };
    Some((
        u32::try_from(width).ok()?.max(1),
        u32::try_from(height).ok()?.max(1),
    ))
}

fn background_image_mode(fill: BackgroundImageFill) -> ui::BackgroundImageMode {
    match fill {
        BackgroundImageFill::Plain => ui::BackgroundImageMode::Plain,
        BackgroundImageFill::Scale => ui::BackgroundImageMode::Scale,
        BackgroundImageFill::Tile => ui::BackgroundImageMode::Tile,
    }
}

fn background_image_alignment(anchor: BackgroundImageAnchor) -> ui::BackgroundImageAlignment {
    match anchor {
        BackgroundImageAnchor::TopLeft => ui::BackgroundImageAlignment::TopLeft,
        BackgroundImageAnchor::TopCenter => ui::BackgroundImageAlignment::TopCenter,
        BackgroundImageAnchor::TopRight => ui::BackgroundImageAlignment::TopRight,
        BackgroundImageAnchor::CenterLeft => ui::BackgroundImageAlignment::CenterLeft,
        BackgroundImageAnchor::Center => ui::BackgroundImageAlignment::Center,
        BackgroundImageAnchor::CenterRight => ui::BackgroundImageAlignment::CenterRight,
        BackgroundImageAnchor::BottomLeft => ui::BackgroundImageAlignment::BottomLeft,
        BackgroundImageAnchor::BottomCenter => ui::BackgroundImageAlignment::BottomCenter,
        BackgroundImageAnchor::BottomRight => ui::BackgroundImageAlignment::BottomRight,
    }
}

fn validate_background_image_path(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!("Background image path must be absolute: {}", path.display());
    }
    Ok(())
}

fn is_supported_background_image(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    SUPPORTED_BACKGROUND_IMAGE_EXTENSIONS
        .iter()
        .any(|supported| extension.eq_ignore_ascii_case(supported))
}

async fn load_background_image(
    fs: Arc<dyn Fs>,
    layer: BackgroundImageLayerSettings,
    cached_key: Option<BackgroundImageKey>,
    cover_target: Option<CoverTarget>,
) -> Result<BackgroundImageLoad> {
    validate_background_image_path(&layer.path)?;
    let metadata = fs
        .metadata(&layer.path)
        .await
        .with_context(|| format!("Failed to read background image {}", layer.path.display()))?
        .with_context(|| format!("Background image not found: {}", layer.path.display()))?;
    if metadata.is_dir {
        bail!("Background image is a directory: {}", layer.path.display());
    }
    let key = BackgroundImageKey::new(&layer, metadata.mtime, cover_target);
    if cached_key.as_ref() == Some(&key) {
        return Ok(BackgroundImageLoad::Unchanged);
    }
    let mut decoded_images = DECODED_BACKGROUND_IMAGES.lock().await;
    decoded_images.retain(|_, image| image.strong_count() > 0);
    if let Some(image) = decoded_images.get(&key).and_then(Weak::upgrade) {
        return Ok(BackgroundImageLoad::Loaded(LoadedBackgroundImage {
            key,
            image,
        }));
    }
    let bytes = fs
        .load_bytes(&layer.path)
        .await
        .with_context(|| format!("Failed to read background image {}", layer.path.display()))?;
    let image = decode_background_image(&bytes, &layer, cover_target)
        .with_context(|| format!("Failed to decode background image {}", layer.path.display()))?;
    let image = Arc::new(image);
    decoded_images.insert(key.clone(), Arc::downgrade(&image));
    Ok(BackgroundImageLoad::Loaded(LoadedBackgroundImage {
        key,
        image,
    }))
}

fn decode_background_image(
    bytes: &[u8],
    layer: &BackgroundImageLayerSettings,
    cover_target: Option<CoverTarget>,
) -> Result<RenderImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("reading image format")?;
    if reader.format().is_none()
        && let Ok(format) = ImageFormat::from_path(&layer.path)
    {
        reader.set_format(format);
    }
    let decoder = reader.into_decoder().context("creating image decoder")?;
    let mut image = decode_oriented_image(decoder)?;
    if image.width() == 0 || image.height() == 0 {
        bail!("image has no pixels");
    }
    let cover = cover_target
        .filter(|_| layer.fill == BackgroundImageFill::Scale)
        .and_then(|target| cover_dimensions((image.width(), image.height()), target))
        .filter(|&(width, height)| width.max(height) <= MAX_BACKGROUND_IMAGE_SIDE);
    if let Some((width, height)) = cover {
        image = image.resize_exact(width, height, image::imageops::FilterType::Triangle);
    } else if image.width().max(image.height()) > MAX_BACKGROUND_IMAGE_SIDE {
        image = image.resize(
            MAX_BACKGROUND_IMAGE_SIDE,
            MAX_BACKGROUND_IMAGE_SIDE,
            image::imageops::FilterType::Triangle,
        );
    }
    if layer.flip_horizontal {
        image = image.fliph();
    }
    if layer.flip_vertical {
        image = image.flipv();
    }
    let mut buffer = image.into_rgba8();
    if layer.fill == BackgroundImageFill::Tile {
        buffer = expand_for_tiling(buffer);
    }
    let opacity = layer.opacity.min(settings::BackgroundImageOpacity::MAX);
    for pixel in buffer.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        let alpha = (u32::from(pixel[3]) * opacity + 50) / 100;
        pixel[3] = u8::try_from(alpha).unwrap_or(u8::MAX);
    }
    Ok(RenderImage::new(vec![image::Frame::new(buffer)]))
}

fn decode_oriented_image(mut decoder: impl ImageDecoder) -> Result<DynamicImage> {
    let orientation = decoder.orientation().context("reading orientation")?;
    let mut image = DynamicImage::from_decoder(decoder).context("decoding image")?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn expand_for_tiling(buffer: RgbaImage) -> RgbaImage {
    let (width, height) = buffer.dimensions();
    let expanded_width = tiled_length(width);
    let expanded_height = tiled_length(height);
    if expanded_width == width && expanded_height == height {
        return buffer;
    }
    RgbaImage::from_fn(expanded_width, expanded_height, |x, y| {
        *buffer.get_pixel(x % width, y % height)
    })
}

fn tiled_length(length: u32) -> u32 {
    if length == 0 || length >= MIN_TILED_BACKGROUND_IMAGE_SIDE {
        return length;
    }
    length * MIN_TILED_BACKGROUND_IMAGE_SIDE.div_ceil(length)
}

#[cfg(test)]
mod tests {
    use std::{
        io::Cursor,
        path::{Path, PathBuf},
    };

    use gpui::{App, DevicePixels, Size};
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use settings::{
        BackgroundImageAnchor, BackgroundImageFill, LocalSettingsKind, LocalSettingsPath,
        Settings as _, SettingsLocation, SettingsStore, WorktreeId,
    };
    use util::rel_path::RelPath;

    use super::{cover_dimensions, decode_background_image, validate_background_image_path};
    use crate::{BackgroundImageLayerSettings, BackgroundImageSettings};

    fn layer_settings(
        opacity: u32,
        fill: BackgroundImageFill,
        flip_horizontal: bool,
    ) -> BackgroundImageLayerSettings {
        BackgroundImageLayerSettings {
            path: util::paths::home_dir().join("background.png"),
            opacity,
            fill,
            anchor: BackgroundImageAnchor::Center,
            flip_horizontal,
            flip_vertical: false,
        }
    }

    fn layer_path(layer: &BackgroundImageLayerSettings) -> PathBuf {
        layer.path.clone()
    }

    fn encode_png(source: RgbaImage) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(source);
        let mut bytes = Vec::new();
        let mut cursor = Cursor::new(&mut bytes);
        let result = image.write_to(&mut cursor, ImageFormat::Png);
        assert!(result.is_ok(), "encoding the test image should succeed");
        bytes
    }

    #[gpui::test]
    fn test_background_image_settings_without_path_are_disabled(cx: &mut App) {
        let mut store = SettingsStore::test(cx);
        assert_eq!(
            store.get::<BackgroundImageSettings>(None),
            &BackgroundImageSettings::default()
        );

        let result = store.set_user_settings(
            r#"{
                "background_image": {
                    "editor_and_tools": { "path": "   ", "opacity": 40 },
                    "empty_frame": { "opacity": 40 }
                }
            }"#,
            cx,
        );
        assert!(result.result().is_ok());
        assert_eq!(
            store.get::<BackgroundImageSettings>(None),
            &BackgroundImageSettings::default()
        );
    }

    #[gpui::test]
    fn test_background_image_settings_expand_home_and_clamp_opacity(cx: &mut App) {
        let mut store = SettingsStore::test(cx);
        let result = store.set_user_settings(
            r#"{
                "background_image": {
                    "editor_and_tools": {
                        "path": "~/Pictures/background.png",
                        "opacity": 250,
                        "fill": "tile",
                        "flip_vertical": true
                    }
                }
            }"#,
            cx,
        );
        assert!(result.result().is_ok());

        let home_dir = util::paths::home_dir();
        let settings = store.get::<BackgroundImageSettings>(None);
        assert_eq!(
            settings.editor_and_tools,
            Some(BackgroundImageLayerSettings {
                path: home_dir.join("Pictures").join("background.png"),
                opacity: 100,
                fill: BackgroundImageFill::Tile,
                anchor: BackgroundImageAnchor::Center,
                flip_horizontal: false,
                flip_vertical: true,
            })
        );
        assert_eq!(settings.empty_frame, None);
    }

    #[gpui::test]
    fn test_background_image_project_settings_override_user_settings(cx: &mut App) {
        let mut store = SettingsStore::test(cx);
        let worktree_id = WorktreeId::from_usize(1);
        let user_result = store.set_user_settings(
            r#"{ "background_image": { "editor_and_tools": { "path": "~/user.png" } } }"#,
            cx,
        );
        assert!(user_result.result().is_ok());
        let project_settings =
            r#"{ "background_image": { "editor_and_tools": { "path": "~/project.png" } } }"#;
        let project_result = store.set_local_settings(
            worktree_id,
            LocalSettingsPath::InWorktree(RelPath::empty().into()),
            LocalSettingsKind::Settings,
            Some(project_settings),
            cx,
        );
        assert!(project_result.is_ok());
        cx.set_global(store);

        let home_dir = util::paths::home_dir();
        let location = SettingsLocation {
            worktree_id,
            path: RelPath::empty(),
        };
        let project = BackgroundImageSettings::get(Some(location), cx);
        let project_path = project.editor_and_tools.as_ref().map(layer_path);
        assert_eq!(project_path, Some(home_dir.join("project.png")));

        let user = BackgroundImageSettings::get_global(cx);
        let user_path = user.editor_and_tools.as_ref().map(layer_path);
        assert_eq!(user_path, Some(home_dir.join("user.png")));
    }

    #[test]
    fn test_background_image_relative_path_is_rejected() {
        let relative = validate_background_image_path(Path::new("images/background.png"));
        assert!(relative.is_err());

        let absolute_path = util::paths::home_dir().join("background.png");
        assert!(validate_background_image_path(&absolute_path).is_ok());
    }

    #[test]
    fn test_background_image_decode_flips_and_scales_alpha() {
        let mut source = RgbaImage::new(2, 1);
        source.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        source.put_pixel(1, 0, Rgba([0, 255, 0, 200]));
        let image = decode_background_image(
            &encode_png(source),
            &layer_settings(50, BackgroundImageFill::Scale, true),
            None,
        );

        let bytes = image
            .as_ref()
            .ok()
            .and_then(|image| image.as_bytes(0).map(<[u8]>::to_vec));
        assert_eq!(bytes, Some(vec![0, 255, 0, 100, 0, 0, 255, 128]));
    }

    #[test]
    fn test_background_image_tile_expansion_reaches_minimum_side() {
        let bytes = encode_png(RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255])));

        let tiled = decode_background_image(
            &bytes,
            &layer_settings(100, BackgroundImageFill::Tile, false),
            None,
        );
        assert_eq!(
            tiled.as_ref().ok().map(|image| image.size(0)),
            Some(Size {
                width: DevicePixels(258),
                height: DevicePixels(256),
            })
        );

        let scaled = decode_background_image(
            &bytes,
            &layer_settings(100, BackgroundImageFill::Scale, false),
            None,
        );
        assert_eq!(
            scaled.as_ref().ok().map(|image| image.size(0)),
            Some(Size {
                width: DevicePixels(3),
                height: DevicePixels(2),
            })
        );
    }

    #[test]
    fn test_background_image_cover_dimensions_keep_aspect_and_cover_target() {
        assert_eq!(
            cover_dimensions((5120, 2880), (2560, 1440)),
            Some((2560, 1440))
        );
        assert_eq!(
            cover_dimensions((4000, 3000), (1000, 1000)),
            Some((1334, 1000))
        );
        assert_eq!(
            cover_dimensions((3000, 4000), (1000, 1000)),
            Some((1000, 1334))
        );

        let cases = [
            ((5120, 2880), (3024, 1964)),
            ((6000, 4000), (2560, 1600)),
            ((1001, 501), (500, 250)),
            ((3, 2), (1, 1)),
            ((9999, 7), (3, 5)),
        ];
        for (source, target) in cases {
            let Some((width, height)) = cover_dimensions(source, target) else {
                panic!("{source:?} should shrink to cover {target:?}");
            };
            assert!(width >= target.0 && height >= target.1);
            assert!(width <= source.0 && height <= source.1);
            let tolerance = u64::from(source.0.max(source.1));
            assert!(
                (u64::from(width) * u64::from(source.1))
                    .abs_diff(u64::from(height) * u64::from(source.0))
                    <= tolerance,
                "{source:?} to {width}x{height} should keep the aspect ratio"
            );
        }
    }

    #[test]
    fn test_background_image_cover_dimensions_round_up() {
        assert_eq!(cover_dimensions((1001, 501), (500, 250)), Some((500, 251)));
        assert_eq!(cover_dimensions((9999, 7), (3, 5)), Some((7143, 5)));
    }

    #[test]
    fn test_background_image_cover_dimensions_never_upscale() {
        assert_eq!(cover_dimensions((100, 100), (200, 200)), None);
        assert_eq!(cover_dimensions((100, 100), (100, 100)), None);
        assert_eq!(cover_dimensions((4000, 500), (1000, 1000)), None);
        assert_eq!(cover_dimensions((500, 4000), (1000, 1000)), None);
        assert_eq!(cover_dimensions((0, 100), (50, 50)), None);
        assert_eq!(cover_dimensions((100, 100), (0, 50)), None);
    }

    #[test]
    fn test_background_image_decode_scale_downsizes_to_cover_target() {
        let bytes = encode_png(RgbaImage::from_pixel(400, 200, Rgba([10, 20, 30, 255])));

        let image = decode_background_image(
            &bytes,
            &layer_settings(100, BackgroundImageFill::Scale, false),
            Some((100, 100)),
        );
        assert_eq!(
            image.as_ref().ok().map(|image| image.size(0)),
            Some(Size {
                width: DevicePixels(200),
                height: DevicePixels(100),
            })
        );

        let not_larger = decode_background_image(
            &bytes,
            &layer_settings(100, BackgroundImageFill::Scale, false),
            Some((400, 200)),
        );
        assert_eq!(
            not_larger.as_ref().ok().map(|image| image.size(0)),
            Some(Size {
                width: DevicePixels(400),
                height: DevicePixels(200),
            })
        );
    }

    #[test]
    fn test_background_image_decode_plain_and_tile_ignore_cover_target() {
        let wide = encode_png(RgbaImage::from_pixel(400, 200, Rgba([10, 20, 30, 255])));
        let plain = decode_background_image(
            &wide,
            &layer_settings(100, BackgroundImageFill::Plain, false),
            Some((100, 100)),
        );
        assert_eq!(
            plain.as_ref().ok().map(|image| image.size(0)),
            Some(Size {
                width: DevicePixels(400),
                height: DevicePixels(200),
            })
        );

        let small = encode_png(RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255])));
        let tiled = decode_background_image(
            &small,
            &layer_settings(100, BackgroundImageFill::Tile, false),
            Some((1, 1)),
        );
        assert_eq!(
            tiled.as_ref().ok().map(|image| image.size(0)),
            Some(Size {
                width: DevicePixels(258),
                height: DevicePixels(256),
            })
        );
    }
}
