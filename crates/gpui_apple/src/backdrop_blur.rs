//! The blur of the renderer's own glass.
//!
//! A backdrop shows what is already drawn behind it, blurred. The blur is a dual filter, the
//! "dual Kawase" blur that Android's and KDE's compositors use for the same job (Bjørge,
//! "Bandwidth-efficient rendering", SIGGRAPH 2015): a pyramid of copies of the frame, each half the
//! size of the one above, is filtered on the way down and again on the way back up. Its cost does
//! not grow with the radius, since a wider blur only adds a smaller level, and every pass reads the
//! frame through a 4x4 footprint, so content that moves under the glass changes the blur smoothly
//! instead of shimmering.
//!
//! The frame is blurred once for all the backdrops that share a draw order and a radius and do not
//! overlap, not once per backdrop: the scene draws backdrops before anything else of their order,
//! so they all blur the same pixels. Every blur has to end the frame's render pass, because a pass
//! cannot sample what it draws into, and the next pass loads the whole frame back, so the number
//! of passes is what the glass costs.

use anyhow::{Context as _, Result, bail};
use gpui::{Backdrop, Bounds, DevicePixels, ScaledPixels, Size, size};
use metal::MTLPixelFormat;
use std::mem;
use std::ops::Range;

/// Levels below the frame, which is level 0. Each level doubles how far the blur reaches; six
/// reach a standard deviation of about 97 device pixels, beyond the largest `glass.blur_radius`.
const MAX_LEVELS: usize = 6;
const MIN_LEVELS: usize = 2;
/// How far from the centre the downsampling filter samples, in source pixels. Sampling closer
/// leaves the filter narrower than the spacing of its output pixels, which aliases content that
/// moves under the glass.
const DOWN_OFFSET: f32 = 1.;
const UP_OFFSET_MIN: f32 = 0.5;
const UP_OFFSET_STEP: f32 = 0.25;
const UP_OFFSET_STEPS: usize = 7;
/// The standard deviation, in device pixels, of a pyramid with `MIN_LEVELS + row` levels for each
/// upsampling offset from `UP_OFFSET_MIN` in steps of `UP_OFFSET_STEP`. The impulse response of
/// the filters in `shaders.metal`; `the_sigma_table_matches_the_filters` recomputes it.
const SIGMA_TABLE: [[f32; UP_OFFSET_STEPS]; MAX_LEVELS - MIN_LEVELS + 1] = [
    [2.72, 3.07, 3.48, 3.84, 4.25, 4.77, 5.30],
    [5.84, 6.64, 7.58, 8.41, 9.35, 10.52, 11.72],
    [11.86, 13.52, 15.45, 17.17, 19.10, 21.51, 23.97],
    [23.82, 27.16, 31.05, 34.51, 38.39, 43.25, 48.21],
    [47.69, 54.37, 62.17, 69.10, 76.89, 86.62, 96.55],
];

/// Texels of the next level up that a texel of a level reads when it is upsampled: the widest
/// tap (the largest offset, 2) and the texel a bilinear sample reaches past it.
const UP_REACH: i32 = 3;
/// Texels of the level above that a texel reads when it is downsampled.
const DOWN_REACH: i32 = 2;
/// Texels of level 1 that the composite reads around a backdrop through its bilinear sample.
const COMPOSITE_REACH: i32 = 1;

const PASS_OFFSET_INDEX: u64 = 0;
const PASS_TEXTURE_INDEX: u64 = 0;
const COMPOSITE_VERTICES_INDEX: u64 = 0;
const COMPOSITE_BACKDROP_INDEX: u64 = 1;
const COMPOSITE_VIEWPORT_SIZE_INDEX: u64 = 2;
const COMPOSITE_TEXTURE_INDEX: u64 = 0;
const COMPOSITE_SHARP_TEXTURE_INDEX: u64 = 1;

/// How deep a pyramid is and how wide its upsampling filter is for a blur radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlurPlan {
    /// How many levels the pyramid has below the frame.
    pub levels: usize,
    /// How far from its centre the upsampling filter samples, in texels of the smaller level.
    pub up_offset: f32,
}

impl BlurPlan {
    /// The plan that blurs with a Gaussian-like spread of `blur_radius / 2` standard deviations,
    /// which is how the Gaussian blur this one replaced read the radius, so the `glass.blur_radius`
    /// setting keeps its look. The shallowest pyramid that reaches that far is used, and its
    /// upsampling offset is interpolated to land on it.
    pub fn for_radius(blur_radius: ScaledPixels) -> Self {
        let sigma = blur_radius.0 / 2.;
        let mut plan = Self {
            levels: MAX_LEVELS,
            up_offset: UP_OFFSET_MIN + UP_OFFSET_STEP * (UP_OFFSET_STEPS - 1) as f32,
        };
        let rows = SIGMA_TABLE.iter().zip(MIN_LEVELS..);
        for (row, levels) in rows {
            let reaches = row.last().is_some_and(|widest| sigma <= *widest);
            if reaches {
                plan = Self {
                    levels,
                    up_offset: up_offset_for(row, sigma),
                };
                break;
            }
        }
        plan
    }
}

fn up_offset_for(row: &[f32; UP_OFFSET_STEPS], sigma: f32) -> f32 {
    let segment = row
        .windows(2)
        .enumerate()
        .find_map(|(step, pair)| match *pair {
            [narrow, wide] if sigma <= wide => {
                let fraction = ((sigma - narrow) / (wide - narrow)).clamp(0., 1.);
                Some(UP_OFFSET_MIN + UP_OFFSET_STEP * (step as f32 + fraction))
            }
            _ => None,
        });
    segment.unwrap_or(UP_OFFSET_MIN)
}

/// A rectangle of texels in one level of the pyramid; `right` and `bottom` are exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TexelRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl TexelRect {
    fn expanded(self, texels: i32) -> Self {
        Self {
            left: self.left - texels,
            top: self.top - texels,
            right: self.right + texels,
            bottom: self.bottom + texels,
        }
    }

    /// The texels of the next level up that overlap this rectangle.
    fn halved(self) -> Self {
        Self {
            left: self.left.div_euclid(2),
            top: self.top.div_euclid(2),
            right: (self.right + 1).div_euclid(2),
            bottom: (self.bottom + 1).div_euclid(2),
        }
    }

    fn doubled(self) -> Self {
        Self {
            left: self.left * 2,
            top: self.top * 2,
            right: self.right * 2,
            bottom: self.bottom * 2,
        }
    }

    fn clamped_to(self, level_size: Size<i32>) -> Option<Self> {
        let clamped = Self {
            left: self.left.max(0),
            top: self.top.max(0),
            right: self.right.min(level_size.width),
            bottom: self.bottom.min(level_size.height),
        };
        (clamped.left < clamped.right && clamped.top < clamped.bottom).then_some(clamped)
    }

    fn touches(&self, other: &Self) -> bool {
        self.left <= other.right
            && other.left <= self.right
            && self.top <= other.bottom
            && other.top <= self.bottom
    }

    fn union(self, other: &Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn scissor(&self) -> metal::MTLScissorRect {
        metal::MTLScissorRect {
            x: self.left as u64,
            y: self.top as u64,
            width: (self.right - self.left) as u64,
            height: (self.bottom - self.top) as u64,
        }
    }
}

/// The size of level `level` (1 is half the frame) for a frame of `viewport`.
fn level_size(viewport: Size<DevicePixels>, level: usize) -> Size<i32> {
    let divisor = 1 << level;
    size(
        (viewport.width.0 + divisor - 1) / divisor,
        (viewport.height.0 + divisor - 1) / divisor,
    )
}

/// The texels each pass has to compute for one backdrop to come out right: reading a pass's
/// output outside its rectangle would see stale texels, so each rectangle is the area the next
/// pass reads, grown by that pass's reach.
#[derive(Debug, PartialEq, Eq)]
struct BlurRegions {
    /// `down[i]` is what the pass that writes level `i + 1` from level `i` computes.
    down: Vec<TexelRect>,
    /// `up[i]` is what the pass that writes level `i + 1` from level `i + 2` computes, which
    /// replaces the level's downsampled texels; the deepest level is only downsampled.
    up: Vec<TexelRect>,
}

impl BlurRegions {
    fn new(
        visible: Bounds<ScaledPixels>,
        levels: usize,
        composite_reach: i32,
        viewport: Size<DevicePixels>,
    ) -> Option<Self> {
        let first_level = TexelRect {
            left: (visible.origin.x.0 / 2.).floor() as i32 - composite_reach,
            top: (visible.origin.y.0 / 2.).floor() as i32 - composite_reach,
            right: ((visible.origin.x.0 + visible.size.width.0) / 2.).ceil() as i32
                + composite_reach,
            bottom: ((visible.origin.y.0 + visible.size.height.0) / 2.).ceil() as i32
                + composite_reach,
        }
        .clamped_to(level_size(viewport, 1))?;

        let mut up = Vec::with_capacity(levels.saturating_sub(1));
        let mut needed = first_level;
        if levels > 1 {
            up.push(first_level);
            for level in 2..levels {
                needed = needed
                    .halved()
                    .expanded(UP_REACH)
                    .clamped_to(level_size(viewport, level))?;
                up.push(needed);
            }
            needed = needed
                .halved()
                .expanded(UP_REACH)
                .clamped_to(level_size(viewport, levels))?;
        }

        let mut down = vec![needed];
        let shallower_levels = (1..levels).rev();
        for level in shallower_levels {
            needed = needed
                .doubled()
                .expanded(DOWN_REACH)
                .clamped_to(level_size(viewport, level))?;
            down.push(needed);
        }
        down.reverse();
        Some(Self { down, up })
    }
}

/// Merges rectangles that touch into their bounding boxes, so the passes of backdrops whose
/// areas meet write each texel once: a pass that computes garbage past the edge of its own
/// rectangle could otherwise overwrite correct texels that another backdrop's rectangle holds.
fn merged(rectangles: impl IntoIterator<Item = TexelRect>) -> Vec<TexelRect> {
    let mut disjoint: Vec<TexelRect> = Vec::new();
    for rectangle in rectangles {
        let mut growing = rectangle;
        loop {
            let before = disjoint.len();
            disjoint.retain(|other| {
                let absorbed = other.touches(&growing);
                if absorbed {
                    growing = growing.union(other);
                }
                !absorbed
            });
            if disjoint.len() == before {
                break;
            }
        }
        disjoint.push(growing);
    }
    disjoint
}

fn visible_bounds(backdrop: &Backdrop) -> Bounds<ScaledPixels> {
    backdrop.bounds.intersect(&backdrop.content_mask.bounds)
}

/// Splits backdrops, sorted by draw order, into the runs that blur together: the same draw order
/// and radius, and no two overlapping. Backdrops that overlap draw one after the other, so the
/// later one blurs the earlier one's glass.
pub(crate) fn groups(backdrops: &[Backdrop]) -> Vec<Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    let candidates = backdrops.iter().enumerate();
    for (index, backdrop) in candidates {
        let group = backdrops.get(start..index).unwrap_or_default();
        let joins = group.iter().all(|member| {
            member.order == backdrop.order
                && member.blur_radius == backdrop.blur_radius
                && !visible_bounds(member).intersects(&visible_bounds(backdrop))
        });
        if !joins {
            groups.push(start..index);
            start = index;
        }
    }
    if start < backdrops.len() {
        groups.push(start..backdrops.len());
    }
    groups
}

/// What one group of backdrops needs blurred: the pyramid it blurs through and the rectangles
/// each of its passes computes.
pub(crate) struct PreparedBlur {
    plan: BlurPlan,
    down: Vec<Vec<TexelRect>>,
    up: Vec<Vec<TexelRect>>,
}

struct Pyramid {
    viewport: Size<DevicePixels>,
    /// `levels[i]` is level `i + 1`: half the frame for `i == 0`, a quarter for the next. Level 1
    /// keeps its downsampled texels, which are barely blurred.
    levels: Vec<metal::Texture>,
    /// The blur itself at half the frame, written by the last upsampling pass so that level 1
    /// stays sharp for the rim of the glass to bend.
    blurred: metal::Texture,
}

pub(crate) struct BackdropBlur {
    down_pipeline_state: metal::RenderPipelineState,
    up_pipeline_state: metal::RenderPipelineState,
    composite_pipeline_state: metal::RenderPipelineState,
    pyramid: Option<Pyramid>,
}

impl BackdropBlur {
    pub fn new(
        down_pipeline_state: metal::RenderPipelineState,
        up_pipeline_state: metal::RenderPipelineState,
        composite_pipeline_state: metal::RenderPipelineState,
    ) -> Self {
        Self {
            down_pipeline_state,
            up_pipeline_state,
            composite_pipeline_state,
            pyramid: None,
        }
    }

    /// Plans the blur of `group`, one run of [`groups`], and makes sure the pyramid exists.
    /// Returns `None` when nothing of the group is visible, so the frame's render pass need not
    /// end.
    pub fn prepare(
        &mut self,
        device: &metal::DeviceRef,
        group: &[Backdrop],
        viewport: Size<DevicePixels>,
    ) -> Option<PreparedBlur> {
        let plan = BlurPlan::for_radius(group.first()?.blur_radius);
        // The rim samples the blur outside the backdrop by up to its lens depth.
        let widest_lens = group
            .iter()
            .map(|backdrop| backdrop.lens.0)
            .fold(0., f32::max);
        let composite_reach = COMPOSITE_REACH + (widest_lens / 2.).ceil() as i32;
        let regions = group
            .iter()
            .filter(|backdrop| backdrop.blur_radius.0 > 0. && backdrop.opacity > 0.)
            .filter_map(|backdrop| {
                let visible = visible_bounds(backdrop);
                if visible.is_empty() {
                    return None;
                }
                BlurRegions::new(visible, plan.levels, composite_reach, viewport)
            })
            .collect::<Vec<_>>();
        if regions.is_empty() {
            return None;
        }

        let pyramid_fits = self.pyramid.as_ref().is_some_and(|pyramid| {
            pyramid.viewport == viewport && pyramid.levels.len() >= plan.levels
        });
        if !pyramid_fits {
            let levels = (1..=plan.levels.max(MIN_LEVELS))
                .map(|level| new_level_texture(device, level_size(viewport, level)))
                .collect();
            self.pyramid = Some(Pyramid {
                viewport,
                levels,
                blurred: new_level_texture(device, level_size(viewport, 1)),
            });
        }

        let down = (0..plan.levels)
            .map(|pass| {
                merged(
                    regions
                        .iter()
                        .filter_map(|regions| regions.down.get(pass).copied()),
                )
            })
            .collect();
        let up = (0..plan.levels.saturating_sub(1))
            .map(|pass| {
                merged(
                    regions
                        .iter()
                        .filter_map(|regions| regions.up.get(pass).copied()),
                )
            })
            .collect();
        Some(PreparedBlur { plan, down, up })
    }

    /// Encodes the passes that blur `frame` into the pyramid. Fails, having encoded only some of
    /// the passes, if the pyramid is missing a level, which [`Self::prepare`] makes sure it is not.
    pub fn encode(
        &self,
        prepared: &PreparedBlur,
        frame: &metal::TextureRef,
        command_buffer: &metal::CommandBufferRef,
    ) -> Result<()> {
        let pyramid = self
            .pyramid
            .as_ref()
            .context("the backdrop blur has no pyramid")?;
        #[cfg(test)]
        poison_levels(pyramid, command_buffer);
        let blurred_level = |pass: usize| {
            if pass == 0 {
                Some(&*pyramid.blurred)
            } else {
                pyramid.levels.get(pass).map(|level| &**level)
            }
        };
        let downsampling_passes = prepared.down.iter().enumerate();
        for (pass, rectangles) in downsampling_passes {
            let source = match pass.checked_sub(1) {
                Some(above) => pyramid.levels.get(above).map(|level| &**level),
                None => Some(frame),
            };
            let (Some(source), Some(destination)) = (source, pyramid.levels.get(pass)) else {
                bail!("the backdrop blur has no level {} to downsample", pass + 1);
            };
            encode_pass(
                &self.down_pipeline_state,
                source,
                destination,
                rectangles,
                DOWN_OFFSET,
                command_buffer,
            );
        }
        let upsampling_passes = prepared.up.iter().enumerate().rev();
        for (pass, rectangles) in upsampling_passes {
            let (Some(source), Some(destination)) =
                (pyramid.levels.get(pass + 1), blurred_level(pass))
            else {
                bail!("the backdrop blur has no level {} to upsample", pass + 1);
            };
            encode_pass(
                &self.up_pipeline_state,
                source,
                destination,
                rectangles,
                prepared.plan.up_offset,
                command_buffer,
            );
        }
        Ok(())
    }

    /// Draws the blurred frame inside `backdrop`'s rounded bounds.
    pub fn composite(
        &self,
        backdrop: &Backdrop,
        unit_vertices: &metal::BufferRef,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) {
        let Some((blurred, sharp)) = self
            .pyramid
            .as_ref()
            .and_then(|pyramid| Some((&pyramid.blurred, pyramid.levels.first()?)))
        else {
            return;
        };
        command_encoder.set_render_pipeline_state(&self.composite_pipeline_state);
        command_encoder.set_vertex_buffer(COMPOSITE_VERTICES_INDEX, Some(unit_vertices), 0);
        command_encoder.set_vertex_bytes(
            COMPOSITE_BACKDROP_INDEX,
            mem::size_of_val(backdrop) as u64,
            backdrop as *const Backdrop as *const _,
        );
        command_encoder.set_vertex_bytes(
            COMPOSITE_VIEWPORT_SIZE_INDEX,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_bytes(
            COMPOSITE_BACKDROP_INDEX,
            mem::size_of_val(backdrop) as u64,
            backdrop as *const Backdrop as *const _,
        );
        command_encoder.set_fragment_texture(COMPOSITE_TEXTURE_INDEX, Some(blurred));
        command_encoder.set_fragment_texture(COMPOSITE_SHARP_TEXTURE_INDEX, Some(sharp));
        command_encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 6);
    }
}

fn new_level_texture(device: &metal::DeviceRef, level_size: Size<i32>) -> metal::Texture {
    let descriptor = metal::TextureDescriptor::new();
    descriptor.set_width(level_size.width.max(1) as u64);
    descriptor.set_height(level_size.height.max(1) as u64);
    descriptor.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
    descriptor.set_storage_mode(metal::MTLStorageMode::Private);
    descriptor.set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
    device.new_texture(&descriptor)
}

/// Fills every level with magenta, so a pass that reads a texel no earlier pass computed shows it.
/// Without this the levels hold whatever the last frame left, which is usually close enough to
/// pass for the right colour.
#[cfg(test)]
fn poison_levels(pyramid: &Pyramid, command_buffer: &metal::CommandBufferRef) {
    let every_level = pyramid.levels.iter().chain([&pyramid.blurred]);
    for level in every_level {
        let descriptor = metal::RenderPassDescriptor::new();
        if let Some(attachment) = descriptor.color_attachments().object_at(0) {
            attachment.set_texture(Some(level));
            attachment.set_load_action(metal::MTLLoadAction::Clear);
            attachment.set_clear_color(metal::MTLClearColor::new(1., 0., 1., 1.));
            attachment.set_store_action(metal::MTLStoreAction::Store);
        }
        command_buffer
            .new_render_command_encoder(descriptor)
            .end_encoding();
    }
}

/// Draws the filter of `pipeline_state` over each of `rectangles` of `destination`.
///
/// The pass does not load `destination`: what it leaves outside the rectangles is never read, and
/// loading a texture just to write over most of it costs the bandwidth the blur is built to save.
fn encode_pass(
    pipeline_state: &metal::RenderPipelineStateRef,
    source: &metal::TextureRef,
    destination: &metal::TextureRef,
    rectangles: &[TexelRect],
    offset: f32,
    command_buffer: &metal::CommandBufferRef,
) {
    let descriptor = metal::RenderPassDescriptor::new();
    if let Some(attachment) = descriptor.color_attachments().object_at(0) {
        attachment.set_texture(Some(destination));
        attachment.set_load_action(metal::MTLLoadAction::DontCare);
        attachment.set_store_action(metal::MTLStoreAction::Store);
    }
    let command_encoder = command_buffer.new_render_command_encoder(descriptor);
    command_encoder.set_viewport(metal::MTLViewport {
        originX: 0.,
        originY: 0.,
        width: destination.width() as f64,
        height: destination.height() as f64,
        znear: 0.,
        zfar: 1.,
    });
    command_encoder.set_render_pipeline_state(pipeline_state);
    command_encoder.set_fragment_texture(PASS_TEXTURE_INDEX, Some(source));
    command_encoder.set_fragment_bytes(
        PASS_OFFSET_INDEX,
        mem::size_of_val(&offset) as u64,
        &offset as *const f32 as *const _,
    );
    for rectangle in rectangles {
        command_encoder.set_scissor_rect(rectangle.scissor());
        command_encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 3);
    }
    command_encoder.end_encoding();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metal_renderer::{InstanceBufferPool, MetalRenderer};
    use gpui::{ContentMask, Corners, Quad, Scene, hsla, point, size};
    use image::RgbaImage;
    use parking_lot::Mutex;
    use std::sync::Arc;

    fn scaled(value: f32) -> ScaledPixels {
        ScaledPixels(value)
    }

    fn backdrop(order: u32, x: f32, y: f32, width: f32, height: f32) -> Backdrop {
        let bounds = Bounds {
            origin: point(scaled(x), scaled(y)),
            size: size(scaled(width), scaled(height)),
        };
        Backdrop {
            order,
            blur_radius: scaled(40.),
            lens: scaled(0.),
            bounds,
            content_mask: ContentMask {
                bounds: Bounds {
                    origin: point(scaled(0.), scaled(0.)),
                    size: size(scaled(10_000.), scaled(10_000.)),
                },
                corner_radii: Corners::default(),
            },
            corner_radii: Corners::default(),
            tint: gpui::transparent_black(),
            opacity: 1.,
        }
    }

    #[test]
    fn backdrops_of_one_order_that_do_not_overlap_blur_together() {
        let islands = [
            backdrop(2, 0., 0., 100., 400.),
            backdrop(2, 120., 300., 600., 100.),
            backdrop(2, 740., 0., 100., 400.),
        ];
        assert_eq!(
            groups(&islands),
            vec![0..3],
            "side by side at one draw order and radius, they share a blur"
        );
    }

    #[test]
    fn backdrops_that_stack_blur_one_after_the_other() {
        let stacked = [
            backdrop(2, 0., 0., 400., 400.),
            backdrop(3, 100., 100., 100., 100.),
            backdrop(3, 300., 100., 50., 50.),
            backdrop(3, 320., 120., 50., 50.),
        ];
        assert_eq!(
            groups(&stacked),
            vec![0..1, 1..3, 3..4],
            "a different order starts a group, and so does an overlap within one order"
        );
    }

    #[test]
    fn backdrops_with_another_radius_do_not_share_a_pyramid() {
        let mut other = backdrop(2, 500., 0., 100., 100.);
        other.blur_radius = scaled(80.);
        let backdrops = [backdrop(2, 0., 0., 100., 100.), other];
        assert_eq!(
            groups(&backdrops),
            vec![0..1, 1..2],
            "another radius needs a pyramid of its own"
        );
        assert_eq!(
            groups(&[]),
            Vec::<Range<usize>>::new(),
            "no backdrops, no groups"
        );
    }

    #[test]
    fn a_wider_radius_never_blurs_less() {
        let mut previous = BlurPlan::for_radius(scaled(0.));
        for radius in 1..=260 {
            let plan = BlurPlan::for_radius(scaled(radius as f32));
            assert!(
                plan.levels > previous.levels
                    || (plan.levels == previous.levels && plan.up_offset >= previous.up_offset),
                "radius {radius}: {plan:?} blurs less than {previous:?}"
            );
            previous = plan;
        }
    }

    #[test]
    fn the_default_glass_radius_uses_a_pyramid_of_four_levels() {
        // 20 points on a 2x screen.
        let plan = BlurPlan::for_radius(scaled(40.));
        assert_eq!(
            plan.levels, 4,
            "a standard deviation of 20 pixels needs four levels"
        );
        assert!((1.5..=1.75).contains(&plan.up_offset), "{plan:?}");
        assert_eq!(
            BlurPlan::for_radius(scaled(0.)).levels,
            MIN_LEVELS,
            "the shallowest pyramid is the floor"
        );
        assert_eq!(
            BlurPlan::for_radius(scaled(10_000.)).levels,
            MAX_LEVELS,
            "the deepest pyramid is the ceiling"
        );
    }

    #[test]
    fn each_pass_computes_what_the_next_pass_reads() {
        let viewport = size(DevicePixels(3456), DevicePixels(2234));
        let visible = Bounds {
            origin: point(scaled(100.), scaled(1500.)),
            size: size(scaled(3200.), scaled(600.)),
        };
        for levels in MIN_LEVELS..=MAX_LEVELS {
            let regions = BlurRegions::new(visible, levels, COMPOSITE_REACH, viewport)
                .expect("a visible backdrop has regions");
            assert_eq!(regions.down.len(), levels, "every level is downsampled");
            assert_eq!(
                regions.up.len(),
                levels - 1,
                "every level but the deepest is upsampled"
            );
            let passes_with_a_pass_above = regions.down.iter().enumerate().skip(1);
            for (pass, down) in passes_with_a_pass_above {
                let above = regions.down.get(pass - 1).expect("a pass above");
                let reads = down.doubled().expanded(DOWN_REACH);
                let above_level = level_size(viewport, pass);
                assert!(
                    above.left <= reads.left.max(0)
                        && above.top <= reads.top.max(0)
                        && above.right >= reads.right.min(above_level.width)
                        && above.bottom >= reads.bottom.min(above_level.height),
                    "levels {levels}, down pass {pass} reads {reads:?} but level {pass} holds {above:?}"
                );
            }
            let upsampling_passes = regions.up.iter().enumerate();
            for (pass, up) in upsampling_passes {
                let reads = up.halved().expanded(UP_REACH);
                let below = regions
                    .up
                    .get(pass + 1)
                    .or(regions.down.last())
                    .expect("a level below");
                let below_level = level_size(viewport, pass + 2);
                assert!(
                    below.left <= reads.left.max(0)
                        && below.top <= reads.top.max(0)
                        && below.right >= reads.right.min(below_level.width)
                        && below.bottom >= reads.bottom.min(below_level.height),
                    "levels {levels}, up pass {pass} reads {reads:?} but level {} holds {below:?}",
                    pass + 2
                );
            }
        }
    }

    #[test]
    fn regions_stay_inside_their_level_and_a_backdrop_off_the_frame_has_none() {
        let viewport = size(DevicePixels(1001), DevicePixels(777));
        let corner = Bounds {
            origin: point(scaled(900.), scaled(700.)),
            size: size(scaled(500.), scaled(500.)),
        };
        let regions = BlurRegions::new(corner, 4, COMPOSITE_REACH, viewport)
            .expect("the corner is on the frame");
        let every_pass = regions.down.iter().chain(&regions.up).enumerate();
        for (index, rectangle) in every_pass {
            assert!(
                rectangle.left >= 0 && rectangle.top >= 0,
                "{index}: {rectangle:?}"
            );
        }
        let downsampling_passes = regions.down.iter().enumerate();
        for (pass, rectangle) in downsampling_passes {
            let level = level_size(viewport, pass + 1);
            assert!(
                rectangle.right <= level.width && rectangle.bottom <= level.height,
                "down pass {pass}: {rectangle:?} outside {level:?}"
            );
        }
        let away = Bounds {
            origin: point(scaled(5000.), scaled(5000.)),
            size: size(scaled(10.), scaled(10.)),
        };
        assert_eq!(
            BlurRegions::new(away, 4, COMPOSITE_REACH, viewport),
            None,
            "a backdrop off the frame blurs nothing"
        );
    }

    #[test]
    fn rectangles_that_touch_merge_and_the_rest_stay_apart() {
        let rectangle = |left, top, right, bottom| TexelRect {
            left,
            top,
            right,
            bottom,
        };
        let chained = merged([
            rectangle(0, 0, 10, 10),
            rectangle(40, 0, 50, 10),
            rectangle(10, 5, 20, 15),
            rectangle(18, 14, 41, 16),
        ]);
        assert_eq!(
            chained,
            vec![rectangle(0, 0, 50, 16)],
            "rectangles that touch through each other merge into one"
        );

        let apart = merged([rectangle(0, 0, 10, 10), rectangle(11, 0, 20, 10)]);
        assert_eq!(
            apart.len(),
            2,
            "rectangles with a gap between them stay apart"
        );
    }

    /// A plane of colour that the dual filter's passes run on, mirroring `shaders.metal` in
    /// floating point and clamping at the edges as the shaders' sampler does.
    struct Plane {
        width: usize,
        height: usize,
        texels: Vec<[f32; 3]>,
    }

    impl Plane {
        fn from_image(image: &RgbaImage) -> Self {
            Self {
                width: image.width() as usize,
                height: image.height() as usize,
                texels: image
                    .pixels()
                    .map(|pixel| {
                        [
                            pixel.0[0] as f32 / 255.,
                            pixel.0[1] as f32 / 255.,
                            pixel.0[2] as f32 / 255.,
                        ]
                    })
                    .collect(),
            }
        }

        fn empty(width: usize, height: usize) -> Self {
            Self {
                width,
                height,
                texels: vec![[0.; 3]; width * height],
            }
        }

        fn texel(&self, x: isize, y: isize) -> [f32; 3] {
            let x = x.clamp(0, self.width as isize - 1) as usize;
            let y = y.clamp(0, self.height as isize - 1) as usize;
            self.texels
                .get(y * self.width + x)
                .copied()
                .unwrap_or_default()
        }

        /// A bilinear sample at a position in texel-edge units, where texel centres sit at +0.5.
        fn sample(&self, x: f32, y: f32) -> [f32; 3] {
            let (x, y) = (x - 0.5, y - 0.5);
            let (left, top) = (x.floor(), y.floor());
            let (across, down) = (x - left, y - top);
            let (left, top) = (left as isize, top as isize);
            let corners = [
                (self.texel(left, top), (1. - across) * (1. - down)),
                (self.texel(left + 1, top), across * (1. - down)),
                (self.texel(left, top + 1), (1. - across) * down),
                (self.texel(left + 1, top + 1), across * down),
            ];
            corners.iter().fold([0.; 3], |sum, (texel, weight)| {
                [
                    sum[0] + texel[0] * weight,
                    sum[1] + texel[1] * weight,
                    sum[2] + texel[2] * weight,
                ]
            })
        }

        fn weighted(&self, taps: &[(f32, f32, f32)], center: (f32, f32), divisor: f32) -> [f32; 3] {
            let mut sum = [0.; 3];
            for (x, y, weight) in taps {
                let texel = self.sample(center.0 + x, center.1 + y);
                let channels = sum.iter_mut().zip(texel);
                for (total, channel) in channels {
                    *total += channel * weight;
                }
            }
            sum.map(|channel| channel / divisor)
        }

        fn downsampled(&self, offset: f32) -> Self {
            let mut halved = Self::empty(self.width.div_ceil(2), self.height.div_ceil(2));
            let taps = [
                (0., 0., 4.),
                (-offset, -offset, 1.),
                (offset, -offset, 1.),
                (offset, offset, 1.),
                (-offset, offset, 1.),
            ];
            for y in 0..halved.height {
                for x in 0..halved.width {
                    let center = ((x as f32 + 0.5) * 2., (y as f32 + 0.5) * 2.);
                    halved.texels[y * halved.width + x] = self.weighted(&taps, center, 8.);
                }
            }
            halved
        }

        fn upsampled(&self, offset: f32, width: usize, height: usize) -> Self {
            let mut doubled = Self::empty(width, height);
            let spacing = 0.5 * offset;
            let taps = [
                (-2. * spacing, 0., 1.),
                (-spacing, spacing, 2.),
                (0., 2. * spacing, 1.),
                (spacing, spacing, 2.),
                (2. * spacing, 0., 1.),
                (spacing, -spacing, 2.),
                (0., -2. * spacing, 1.),
                (-spacing, -spacing, 2.),
            ];
            for y in 0..height {
                for x in 0..width {
                    let center = ((x as f32 + 0.5) * 0.5, (y as f32 + 0.5) * 0.5);
                    doubled.texels[y * width + x] = self.weighted(&taps, center, 12.);
                }
            }
            doubled
        }
    }

    /// The first level of the pyramid after the whole chain, run over the entire frame.
    fn blurred_first_level(frame: &Plane, plan: BlurPlan) -> Plane {
        let mut levels = vec![frame.downsampled(DOWN_OFFSET)];
        for _ in 1..plan.levels {
            let below = levels.last().map(|level| level.downsampled(DOWN_OFFSET));
            levels.extend(below);
        }
        let mut blurred = levels.pop().unwrap_or_else(|| Plane::empty(1, 1));
        while let Some(above) = levels.pop() {
            blurred = blurred.upsampled(plan.up_offset, above.width, above.height);
        }
        blurred
    }

    fn blurred_frame(frame: &Plane, plan: BlurPlan) -> Plane {
        let first_level = blurred_first_level(frame, plan);
        let mut blurred = Plane::empty(frame.width, frame.height);
        for y in 0..frame.height {
            for x in 0..frame.width {
                blurred.texels[y * frame.width + x] =
                    first_level.sample((x as f32 + 0.5) * 0.5, (y as f32 + 0.5) * 0.5);
            }
        }
        blurred
    }

    /// Content with detail at every scale, so a pass that reads a texel it never computed is
    /// visible in the blur: a gradient under a few hundred small and large rectangles.
    fn content_scene(width: f32, height: f32) -> Scene {
        let mut scene = Scene::default();
        let full = Bounds {
            origin: point(scaled(0.), scaled(0.)),
            size: size(scaled(width), scaled(height)),
        };
        let mask = ContentMask {
            bounds: full,
            corner_radii: Corners::default(),
        };
        let mut seed = 0x2545_f491_u32;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        scene.insert_primitive(Quad {
            bounds: full,
            content_mask: mask,
            background: hsla(0.6, 0.5, 0.15, 1.).into(),
            ..Default::default()
        });
        for _ in 0..500 {
            let (rect_width, rect_height) = (3. + random() * 90., 3. + random() * 60.);
            scene.insert_primitive(Quad {
                bounds: Bounds {
                    origin: point(
                        scaled(random() * (width - rect_width)),
                        scaled(random() * (height - rect_height)),
                    ),
                    size: size(scaled(rect_width), scaled(rect_height)),
                },
                content_mask: mask,
                background: hsla(random(), 0.9, 0.2 + random() * 0.6, 1.).into(),
                ..Default::default()
            });
        }
        scene
    }

    fn glass(
        scene: &mut Scene,
        bounds: Bounds<ScaledPixels>,
        mask_bounds: Bounds<ScaledPixels>,
        lens: f32,
    ) {
        scene.insert_primitive(Backdrop {
            order: 0,
            blur_radius: scaled(40.),
            lens: scaled(lens),
            bounds,
            content_mask: ContentMask {
                bounds: mask_bounds,
                corner_radii: Corners::default(),
            },
            corner_radii: Corners::default(),
            tint: gpui::transparent_black(),
            opacity: 1.,
        });
    }

    fn rectangle(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds {
            origin: point(scaled(x), scaled(y)),
            size: size(scaled(width), scaled(height)),
        }
    }

    fn render(scene: &mut Scene, width: i32, height: i32) -> RgbaImage {
        scene.finish();
        let mut renderer =
            MetalRenderer::new_headless(Arc::new(Mutex::new(InstanceBufferPool::default())));
        renderer
            .render_scene_to_image(scene, size(DevicePixels(width), DevicePixels(height)))
            .expect("the scene should render")
    }

    /// How far `image` is from `expected` inside `area`, away from its edges: the largest and the
    /// mean difference of any channel, in 8-bit levels.
    fn difference_inside(
        image: &RgbaImage,
        expected: &Plane,
        area: Bounds<ScaledPixels>,
    ) -> (f32, f32) {
        let inset = 2.;
        let (left, top) = (area.origin.x.0 + inset, area.origin.y.0 + inset);
        let (right, bottom) = (
            area.origin.x.0 + area.size.width.0 - inset,
            area.origin.y.0 + area.size.height.0 - inset,
        );
        let (mut largest, mut total, mut count) = (0f32, 0f32, 0u32);
        for y in top.ceil() as u32..bottom.floor() as u32 {
            for x in left.ceil() as u32..right.floor() as u32 {
                let actual = image.get_pixel(x, y).0;
                let wanted = expected.texel(x as isize, y as isize);
                for channel in 0..3 {
                    let difference =
                        ((actual[channel] as f32 / 255. - wanted[channel]) * 255.).abs();
                    largest = largest.max(difference);
                    total += difference;
                    count += 1;
                }
            }
        }
        (largest, total / count.max(1) as f32)
    }

    fn assert_blurs_like_the_filters(width: i32, height: i32, glasses: &[Bounds<ScaledPixels>]) {
        let (frame_width, frame_height) = (width as f32, height as f32);
        let frame = render(&mut content_scene(frame_width, frame_height), width, height);
        let expected = blurred_frame(
            &Plane::from_image(&frame),
            BlurPlan::for_radius(scaled(40.)),
        );

        let mut scene = content_scene(frame_width, frame_height);
        // A layer gives the glass one draw order, as the islands of the docks have.
        scene.push_layer(rectangle(0., 0., frame_width, frame_height));
        for bounds in glasses {
            glass(
                &mut scene,
                *bounds,
                rectangle(0., 0., frame_width, frame_height),
                0.,
            );
        }
        scene.pop_layer();
        let image = render(&mut scene, width, height);
        assert_eq!(
            groups(&scene.backdrops),
            vec![0..glasses.len()],
            "the glass blurs together"
        );
        for bounds in glasses {
            let (largest, mean) = difference_inside(&image, &expected, *bounds);
            assert!(
                largest <= 3. && mean <= 0.6,
                "the glass at {bounds:?} of a {width}x{height} frame differs from the filters by \
                 up to {largest} levels, {mean} on average"
            );
        }
    }

    #[test]
    fn the_gpu_blurs_like_the_filters_do_on_the_cpu() {
        assert_blurs_like_the_filters(640, 480, &[rectangle(120., 90., 400., 300.)]);
    }

    #[test]
    fn a_backdrop_at_the_edges_of_an_odd_sized_frame_blurs_like_the_filters() {
        assert_blurs_like_the_filters(
            641,
            479,
            &[
                rectangle(0., 0., 300., 479.),
                rectangle(341., 200., 300., 279.),
            ],
        );
    }

    #[test]
    fn backdrops_that_share_a_pyramid_do_not_spoil_each_others_texels() {
        assert_blurs_like_the_filters(
            800,
            600,
            &[
                rectangle(20., 20., 200., 560.),
                rectangle(240., 400., 320., 180.),
                rectangle(580., 20., 200., 560.),
            ],
        );
    }

    #[test]
    fn a_backdrop_blurs_the_glass_behind_it() {
        let (width, height) = (640, 480);
        let (frame_width, frame_height) = (width as f32, height as f32);
        let lower = rectangle(60., 60., 360., 300.);
        let upper = rectangle(200., 140., 360., 280.);

        let mut under = content_scene(frame_width, frame_height);
        glass(
            &mut under,
            lower,
            rectangle(0., 0., frame_width, frame_height),
            0.,
        );
        let under = render(&mut under, width, height);
        let expected = blurred_frame(
            &Plane::from_image(&under),
            BlurPlan::for_radius(scaled(40.)),
        );

        let mut scene = content_scene(frame_width, frame_height);
        glass(
            &mut scene,
            lower,
            rectangle(0., 0., frame_width, frame_height),
            0.,
        );
        // A quad over the lower glass puts the upper one on a later draw order.
        scene.insert_primitive(Quad {
            bounds: upper,
            content_mask: ContentMask {
                bounds: rectangle(0., 0., frame_width, frame_height),
                corner_radii: Corners::default(),
            },
            background: hsla(0., 0., 0., 0.).into(),
            ..Default::default()
        });
        glass(
            &mut scene,
            upper,
            rectangle(0., 0., frame_width, frame_height),
            0.,
        );
        let image = render(&mut scene, width, height);
        let (largest, mean) = difference_inside(&image, &expected, upper);
        assert!(
            largest <= 3. && mean <= 0.6,
            "the upper glass blurs the lower one: up to {largest} levels, {mean} on average"
        );
    }

    #[test]
    fn the_sigma_table_matches_the_filters() {
        // A point spread through the filters: its standard deviation is how far they blur.
        let rows = SIGMA_TABLE.iter().zip(MIN_LEVELS..);
        for (row, levels) in rows {
            let side = 2 * (6. * row.last().copied().unwrap_or(1.)) as usize;
            let side = side.next_power_of_two().max(64);
            // The deepest pyramid is big enough to be slow, and doubles the one above it.
            let steps: &[usize] = if levels == MAX_LEVELS {
                &[0]
            } else {
                &[0, UP_OFFSET_STEPS / 2, UP_OFFSET_STEPS - 1]
            };
            for &step in steps {
                let Some(expected) = row.get(step) else {
                    continue;
                };
                let up_offset = UP_OFFSET_MIN + UP_OFFSET_STEP * step as f32;
                let mut point_source = Plane::empty(side, side);
                if let Some(center) = point_source.texels.get_mut(side / 2 * side + side / 2) {
                    *center = [1.; 3];
                }
                let spread = blurred_first_level(&point_source, BlurPlan { levels, up_offset });
                let (mut mass, mut mean, mut second) = (0f32, 0f32, 0f32);
                for column in 0..spread.width {
                    let column_mass: f32 = (0..spread.height)
                        .map(|row| spread.texel(column as isize, row as isize)[0])
                        .sum();
                    mass += column_mass;
                    mean += column_mass * column as f32;
                    second += column_mass * (column * column) as f32;
                }
                mean /= mass;
                // Level 1 has half the frame's resolution.
                let sigma = (second / mass - mean * mean).sqrt() * 2.;
                assert!(
                    (sigma - expected).abs() <= expected * 0.02,
                    "{levels} levels at offset {up_offset} blur by {sigma}, the table says {expected}"
                );
            }
        }
    }

    #[test]
    fn the_rim_of_a_backdrop_shows_the_sharp_backdrop_from_outside_it() {
        let (width, height) = (640, 480);
        let (frame_width, frame_height) = (width as f32, height as f32);
        let bounds = rectangle(120., 90., 400., 300.);
        let lens = 16.;
        // The shift fades out over a band 2.5 times as wide as the lens.
        let band = lens * 2.5;

        let frame = render(&mut content_scene(frame_width, frame_height), width, height);
        let frame_plane = Plane::from_image(&frame);
        let first_level = blurred_first_level(&frame_plane, BlurPlan::for_radius(scaled(40.)));
        let sharp_level = frame_plane.downsampled(DOWN_OFFSET);

        let mut flat = content_scene(frame_width, frame_height);
        glass(
            &mut flat,
            bounds,
            rectangle(0., 0., frame_width, frame_height),
            0.,
        );
        let flat = render(&mut flat, width, height);
        let mut bent = content_scene(frame_width, frame_height);
        glass(
            &mut bent,
            bounds,
            rectangle(0., 0., frame_width, frame_height),
            lens,
        );
        let bent = render(&mut bent, width, height);

        let (left, top) = (bounds.origin.x.0, bounds.origin.y.0);
        let (right, bottom) = (left + bounds.size.width.0, top + bounds.size.height.0);
        let (mut largest_error, mut largest_bend) = (0f32, 0f32);
        for y in top as u32 + 2..bottom as u32 - 2 {
            for x in left as u32 + 2..right as u32 - 2 {
                let (center_x, center_y) = (x as f32 + 0.5, y as f32 + 0.5);
                let mut distances = [
                    (center_x - left, (-1., 0.)),
                    (right - center_x, (1., 0.)),
                    (center_y - top, (0., -1.)),
                    (bottom - center_y, (0., 1.)),
                ];
                distances.sort_by(|first, second| first.0.total_cmp(&second.0));
                let [(depth, (normal_x, normal_y)), (next, _), ..] = distances;
                // Near a corner the direction away from the edge turns; the shader follows the
                // rounded distance field there, and this reference only knows straight edges.
                if next < band + 2. {
                    continue;
                }
                let remaining = if depth < band { 1. - depth / band } else { 0. };
                let shift = lens * remaining * remaining;
                let (sample_x, sample_y) = (
                    (center_x + normal_x * shift) * 0.5,
                    (center_y + normal_y * shift) * 0.5,
                );
                let blurred = first_level.sample(sample_x, sample_y);
                let sharp = sharp_level.sample(sample_x, sample_y);
                let expected = [0, 1, 2].map(|channel| {
                    blurred[channel] + (sharp[channel] - blurred[channel]) * remaining
                });
                let actual = bent.get_pixel(x, y).0;
                let unbent = flat.get_pixel(x, y).0;
                for channel in 0..3 {
                    largest_error = largest_error
                        .max((actual[channel] as f32 - expected[channel] * 255.).abs());
                    largest_bend =
                        largest_bend.max((actual[channel] as f32 - unbent[channel] as f32).abs());
                }
            }
        }
        assert!(
            largest_error <= 4.,
            "the rim shows the sharp backdrop from outside the glass as the filters say: {largest_error}"
        );
        assert!(
            largest_bend >= 12.,
            "the lens changes what the rim shows: {largest_bend} levels"
        );
    }
}
