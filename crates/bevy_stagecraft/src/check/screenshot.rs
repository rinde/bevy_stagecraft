//! Screenshot checks: the frame, or one node's rectangle of it, against a
//! picture stored next to the scenario.
//!
//! The baseline is `<base_dir>/<slug of the scenario name>/<name>.png`. A
//! missing baseline is written and fails. A mismatch writes
//! `.diagnostics/<name>.actual.png` and `.diff.png` beside the baselines, and a
//! match removes them; a name may not start with `.`, so no baseline can
//! collide. Update mode (`RIN_UPDATE_SNAPSHOTS`) rewrites the baseline.
//!
//! Pixels need the renderer: without the recorder (feature `record`) the check
//! is skipped, which does not fail the scenario. PNG encoding and decoding need
//! that feature too; the rest is ungated and tested without a GPU.

use alloc::{string::String, vec::Vec};
use core::fmt;
use std::{fs, path::Path, path::PathBuf};

use bevy::{
    camera::visibility::Visibility,
    ecs::{entity::Entity, resource::Resource, world::World},
    math::{Affine2, UVec2, Vec2},
    ui::{ComputedNode, UiGlobalTransform},
};
use serde::{Deserialize, Serialize};

use super::{Check, Verdict, widget};
use crate::targets::{Lookup, Target};

pub(crate) const UPDATE_ENV: &str = "RIN_UPDATE_SNAPSHOTS";
pub(crate) const DEFAULT_COLOUR_TOLERANCE: u8 = 2;
const CHANNELS: usize = 4;
/// A renderer that never answers is an error, not a hang.
const MAX_PICTURE_FRAMES: u32 = 600;
const DIFF_COLOUR: [u8; 4] = [255, 0, 255, 255];
const DIFF_DIM_DIVISOR: u8 = 3;
/// A name may not start with `.`, so no baseline can be this directory or lie
/// in it.
const DIAGNOSTICS_DIR: &str = ".diagnostics";
const ACTUAL_SUFFIX: &str = "actual";
const DIFF_SUFFIX: &str = "diff";

/// Noise a renderer adds is not a regression. In a file:
/// `tolerance: (colour: 4, pixels: 0.01)`, leaving out what is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tolerance {
    pub colour: u8,
    /// The largest share of pixels, from 0 to 1, that may differ.
    pub pixels: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            colour: DEFAULT_COLOUR_TOLERANCE,
            pixels: 0.0,
        }
    }
}

impl Tolerance {
    #[must_use]
    pub const fn exact() -> Self {
        Self {
            colour: 0,
            pixels: 0.0,
        }
    }

    #[must_use]
    pub const fn with_colour(mut self, colour: u8) -> Self {
        self.colour = colour;
        self
    }

    #[must_use]
    pub const fn with_pixels(mut self, pixels: f64) -> Self {
        self.pixels = pixels;
        self
    }

    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub(super) fn problem(&self) -> Option<String> {
        (!self.pixels.is_finite() || !(0.0..=1.0).contains(&self.pixels)).then(|| {
            format!(
                "the share of pixels that may differ must be from 0 to 1, not {}",
                self.pixels
            )
        })
    }
}

impl fmt::Display for Tolerance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "colour {}, {}% of pixels",
            self.colour,
            self.pixels * 100.0
        )
    }
}

pub(super) fn name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        Some("a screenshot needs a name".into())
    } else if name.starts_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
    {
        Some(format!(
            "`{name}` cannot be a file name: it may not start with `.` or hold `/`, `\\` or `:`"
        ))
    } else {
        None
    }
}

/// Everything but letters and digits becomes `-`.
#[must_use]
pub(crate) fn directory_name(scenario: &str) -> String {
    let slug: String = scenario
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "scenario".into()
    } else {
        slug
    }
}

/// A name may not start with `.`, so it never clashes with a baseline.
const OWNER_FILE: &str = ".scenario";

/// `directory_name` is lossy (`A B` and `A-B` are both `a-b`), so the folder
/// records its exact owner in [`OWNER_FILE`] and another name finds it taken. A
/// folder with baselines and no record is adopted by the first scenario to use
/// it; failing to write the record is not an error.
fn claim_directory(folder: &Path, scenario: &str) -> Result<(), String> {
    let owner = folder.join(OWNER_FILE);
    let Ok(existing) = fs::read_to_string(&owner) else {
        if fs::create_dir_all(folder).is_ok() {
            let _ = fs::write(&owner, format!("{scenario}\n"));
        }
        return Ok(());
    };
    let existing = existing.trim_end_matches(['\r', '\n']);
    if existing == scenario {
        Ok(())
    } else {
        Err(format!(
            "the baseline directory {} belongs to the scenario `{existing}`, and `{scenario}` \
             would share it: rename one of them so their names differ in letters or digits",
            folder.display()
        ))
    }
}

/// Starts as `RIN_UPDATE_SNAPSHOTS` says (set and not `0`). Only the recorder
/// has pictures to write.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenshotSettings {
    pub update: bool,
}

impl Default for ScreenshotSettings {
    fn default() -> Self {
        Self {
            update: std::env::var(UPDATE_ENV).is_ok_and(|value| !value.is_empty() && value != "0"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenshotRecord {
    pub name: String,
    pub baseline: PathBuf,
    pub result: ScreenshotResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScreenshotResult {
    Matched,
    Created,
    Updated,
    Mismatch {
        actual: PathBuf,
        diff: Option<PathBuf>,
    },
}

/// Tightly packed 8-bit RGBA, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Picture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

impl Picture {
    #[cfg(test)]
    pub(crate) fn filled(width: u32, height: u32, colour: [u8; 4]) -> Self {
        let rgba = colour
            .iter()
            .copied()
            .cycle()
            .take(width as usize * height as usize * CHANNELS)
            .collect();
        Self {
            width,
            height,
            rgba,
        }
    }

    fn pixel(&self, x: u32, y: u32) -> Option<&[u8]> {
        let start = (y as usize * self.width as usize + x as usize) * CHANNELS;
        self.rgba.get(start..start + CHANNELS)
    }

    /// Clipped to the picture; `None` where nothing is inside.
    fn crop(&self, left_top: UVec2, size: UVec2) -> Option<Self> {
        let right = left_top.x.saturating_add(size.x).min(self.width);
        let bottom = left_top.y.saturating_add(size.y).min(self.height);
        if left_top.x >= right || left_top.y >= bottom {
            return None;
        }
        let mut rgba = Vec::with_capacity((right - left_top.x) as usize * CHANNELS);
        for y in left_top.y..bottom {
            for x in left_top.x..right {
                rgba.extend_from_slice(self.pixel(x, y)?);
            }
        }
        Some(Self {
            width: right - left_top.x,
            height: bottom - left_top.y,
            rgba,
        })
    }

    fn differing(&self, other: &Self, colour: u8) -> usize {
        let (ours, _) = self.rgba.as_chunks::<CHANNELS>();
        let (theirs, _) = other.rgba.as_chunks::<CHANNELS>();
        ours.iter()
            .zip(theirs)
            .filter(|(a, b)| differs(**a, **b, colour))
            .count()
    }

    fn diff(&self, other: &Self, colour: u8) -> Self {
        let mut rgba = Vec::with_capacity(self.rgba.len());
        let (ours, _) = self.rgba.as_chunks::<CHANNELS>();
        let (theirs, _) = other.rgba.as_chunks::<CHANNELS>();
        for (a, b) in ours.iter().zip(theirs) {
            if differs(*a, *b, colour) {
                rgba.extend_from_slice(&DIFF_COLOUR);
            } else {
                rgba.extend(a.iter().take(3).map(|channel| channel / DIFF_DIM_DIVISOR));
                rgba.push(u8::MAX);
            }
        }
        Self {
            width: self.width,
            height: self.height,
            rgba,
        }
    }
}

fn differs(a: [u8; CHANNELS], b: [u8; CHANNELS], colour: u8) -> bool {
    a.iter().zip(b).any(|(a, b)| a.abs_diff(b) > colour)
}

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Idle,
    Wanted,
    Requested,
    Ready(Result<Picture, String>),
}

/// Present only where a renderer is, so a world without it skips the checks.
#[derive(Resource, Debug, Default)]
pub(crate) struct Pictures {
    phase: Phase,
    /// Which request the picture is for, so one that comes back late from a
    /// stopped scenario is not taken for the next one's.
    token: u32,
    waited: u32,
    judgements: Vec<(Check, Judgement)>,
    hidden: Vec<(Entity, Visibility)>,
}

impl Pictures {
    pub(crate) fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.token = self.token.wrapping_add(1);
        self.judgements.clear();
    }

    fn want(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.waited = 0;
        self.phase = Phase::Wanted;
    }

    pub(crate) fn begin_taking(&mut self) -> Option<u32> {
        matches!(self.phase, Phase::Wanted).then(|| {
            self.phase = Phase::Requested;
            self.token
        })
    }

    pub(crate) const fn is_wanted(&self) -> bool {
        matches!(self.phase, Phase::Wanted)
    }

    /// One for another request is dropped.
    pub(crate) fn deliver(&mut self, token: u32, picture: Result<Picture, String>) {
        if token == self.token && matches!(self.phase, Phase::Requested) {
            self.phase = Phase::Ready(picture);
        }
    }

    pub(crate) fn hide(&mut self, hidden: Vec<(Entity, Visibility)>) {
        self.hidden = hidden;
    }

    pub(crate) fn take_hidden(&mut self) -> Vec<(Entity, Visibility)> {
        core::mem::take(&mut self.hidden)
    }

    fn advance(&mut self) -> Option<Result<Picture, String>> {
        match core::mem::take(&mut self.phase) {
            Phase::Idle => {
                self.want();
                None
            }
            Phase::Ready(picture) => Some(picture),
            waiting => {
                self.waited += 1;
                if self.waited > MAX_PICTURE_FRAMES {
                    return Some(Err(format!(
                        "no picture came back after {MAX_PICTURE_FRAMES} frames: is there a \
                         (software) Vulkan driver, and is WGPU_BACKEND set?"
                    )));
                }
                self.phase = waiting;
                None
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Judgement {
    Holds,
    Fails(String),
    Broken(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Readiness {
    Waiting,
    Ready,
}

pub(crate) fn prepare(
    world: &mut World,
    check: &Check,
    directory: Option<&Path>,
    scenario: &str,
    records: &mut Vec<ScreenshotRecord>,
) -> Readiness {
    if !check.has_screenshot() {
        return Readiness::Ready;
    }
    let Some(mut pictures) = world.get_resource_mut::<Pictures>() else {
        return Readiness::Ready;
    };
    let Some(picture) = pictures.advance() else {
        return Readiness::Waiting;
    };
    let update = world
        .get_resource::<ScreenshotSettings>()
        .is_some_and(|settings| settings.update);
    let folder = directory
        .unwrap_or_else(|| Path::new("."))
        .join(directory_name(scenario));
    let claim = claim_directory(&folder, scenario);
    let mut judgements = Vec::new();
    for part in check.parts() {
        let Check::Screenshot {
            name,
            of,
            tolerance,
        } = part
        else {
            continue;
        };
        let judgement = match &picture {
            Ok(_) if claim.is_err() => Judgement::Broken(claim.clone().err().unwrap_or_default()),
            Ok(picture) => {
                let (judgement, record) = judge(
                    world,
                    picture,
                    name,
                    of.as_ref(),
                    *tolerance,
                    &folder,
                    update,
                );
                if let Some(record) = record {
                    records.retain(|earlier| earlier.name != record.name);
                    records.push(record);
                }
                judgement
            }
            Err(message) => Judgement::Broken(message.clone()),
        };
        judgements.push((part.clone(), judgement));
    }
    if let Some(mut pictures) = world.get_resource_mut::<Pictures>() {
        pictures.judgements = judgements;
    }
    Readiness::Ready
}

pub(crate) fn forget(world: &mut World) {
    if let Some(mut pictures) = world.get_resource_mut::<Pictures>() {
        pictures.judgements.clear();
    }
}

pub(super) fn verdict(world: &World, check: &Check) -> Verdict {
    let Some(pictures) = world.get_resource::<Pictures>() else {
        return Verdict::Unavailable(
            "a screenshot needs the renderer, which only the recorder has (feature `record`)"
                .into(),
        );
    };
    match pictures
        .judgements
        .iter()
        .find(|(judged, _)| judged == check)
    {
        Some((_, Judgement::Holds)) => Verdict::Holds,
        Some((_, Judgement::Fails(message))) => Verdict::Fails(message.clone()),
        Some((_, Judgement::Broken(message))) => Verdict::Broken(message.clone()),
        None => Verdict::Broken("no picture was taken for this check".into()),
    }
}

/// A quarter-turned node has corners off by float noise, which would widen the
/// crop by a whole pixel.
const EDGE_EPSILON: f32 = 1e-3;

/// The upright bounding box of the node's box under its complete
/// [`UiGlobalTransform`], widened to whole pixels: a quarter turn swaps width
/// and height, and other turns include pixels outside the node. Clipped here
/// on the left and top, the rest by [`Picture::crop`].
fn node_rectangle(world: &World, node: Entity) -> Option<(UVec2, UVec2)> {
    let computed = world.get::<ComputedNode>(node)?;
    let transform = world.get::<UiGlobalTransform>(node)?.affine();
    Some(bounds(transform, computed.size()))
}

fn bounds(transform: Affine2, size: Vec2) -> (UVec2, UVec2) {
    let half = size / 2.0;
    let (low, high) = [
        Vec2::new(-half.x, -half.y),
        Vec2::new(half.x, -half.y),
        Vec2::new(-half.x, half.y),
        half,
    ]
    .into_iter()
    .map(|corner| transform.transform_point2(corner))
    .fold((Vec2::MAX, Vec2::MIN), |(low, high), point| {
        (low.min(point), high.max(point))
    });
    let top_left = (low + EDGE_EPSILON).floor().max(Vec2::ZERO);
    let bottom_right = (high - EDGE_EPSILON).ceil().max(Vec2::ZERO);
    (
        top_left.as_uvec2(),
        (bottom_right - top_left).max(Vec2::ZERO).as_uvec2(),
    )
}

fn judge(
    world: &World,
    picture: &Picture,
    name: &str,
    of: Option<&Target>,
    tolerance: Tolerance,
    folder: &Path,
    update: bool,
) -> (Judgement, Option<ScreenshotRecord>) {
    if let Some(problem) = name_problem(name).or_else(|| tolerance.problem()) {
        return (Judgement::Broken(problem), None);
    }
    let cropped;
    let actual = if let Some(target) = of {
        let node = match widget::node(world, target) {
            Lookup::Found(node) => node,
            Lookup::Missing(message) => return (Judgement::Fails(message), None),
            Lookup::Ambiguous(message) | Lookup::NotANode(message) => {
                return (Judgement::Broken(message), None);
            }
        };
        let Some((left_top, size)) = node_rectangle(world, node) else {
            return (
                Judgement::Fails(format!("{target:?} has no place on screen yet")),
                None,
            );
        };
        let Some(part) = picture.crop(left_top, size) else {
            return (
                Judgement::Fails(format!("{target:?} is outside the picture")),
                None,
            );
        };
        cropped = part;
        &cropped
    } else {
        picture
    };
    let baseline = folder.join(format!("{name}.png"));
    let outcome = compare(actual, name, &baseline, tolerance, update);
    let record = |result| ScreenshotRecord {
        name: name.to_owned(),
        baseline: baseline.clone(),
        result,
    };
    match outcome {
        Ok(Compared::Matched) => (Judgement::Holds, Some(record(ScreenshotResult::Matched))),
        Ok(Compared::Updated) => (Judgement::Holds, Some(record(ScreenshotResult::Updated))),
        Ok(Compared::Created) => (
            Judgement::Fails(format!(
                "there is no baseline `{}`: this picture was written there; look at it, and run \
                 again to compare",
                baseline.display()
            )),
            Some(record(ScreenshotResult::Created)),
        ),
        Ok(Compared::Differs {
            message,
            actual,
            diff,
        }) => (
            Judgement::Fails(message),
            Some(record(ScreenshotResult::Mismatch { actual, diff })),
        ),
        Err(message) => (Judgement::Broken(message), None),
    }
}

enum Compared {
    Matched,
    Updated,
    Created,
    Differs {
        message: String,
        actual: PathBuf,
        diff: Option<PathBuf>,
    },
}

/// An error is a file that cannot be read or written, no verdict on the
/// picture.
fn compare(
    actual: &Picture,
    name: &str,
    baseline: &Path,
    tolerance: Tolerance,
    update: bool,
) -> Result<Compared, String> {
    // Older `<name>.actual.png` files beside the baselines are never touched:
    // one may be a baseline.
    let diagnostics = baseline.with_file_name(DIAGNOSTICS_DIR);
    let actual_path = diagnostics.join(format!("{name}.{ACTUAL_SUFFIX}.png"));
    let diff_path = diagnostics.join(format!("{name}.{DIFF_SUFFIX}.png"));
    if update {
        write_png(baseline, actual)?;
        remove_stale(&actual_path, &diff_path);
        return Ok(Compared::Updated);
    }
    if !baseline.exists() {
        write_png(baseline, actual)?;
        return Ok(Compared::Created);
    }
    let bytes = fs::read(baseline).map_err(|error| {
        format!(
            "the baseline `{}` cannot be read: {error}",
            baseline.display()
        )
    })?;
    let expected = png::decode(&bytes).map_err(|error| {
        format!(
            "the baseline `{}` is no picture: {error}",
            baseline.display()
        )
    })?;
    if expected.width != actual.width || expected.height != actual.height {
        write_png(&actual_path, actual)?;
        return Ok(Compared::Differs {
            message: format!(
                "the picture is {}x{} pixels, the baseline `{}` {}x{}; the picture is in `{}`",
                actual.width,
                actual.height,
                baseline.display(),
                expected.width,
                expected.height,
                actual_path.display()
            ),
            actual: actual_path,
            diff: None,
        });
    }
    let differing = actual.differing(&expected, tolerance.colour);
    let total = actual.rgba.len() / CHANNELS;
    if within_share(differing, total, tolerance.pixels) {
        remove_stale(&actual_path, &diff_path);
        return Ok(Compared::Matched);
    }
    write_png(&actual_path, actual)?;
    write_png(&diff_path, &expected.diff(actual, tolerance.colour))?;
    Ok(Compared::Differs {
        message: format!(
            "{differing} of {total} pixels differ from the baseline `{}` by more than {} in a \
             channel (up to {}% may); the picture is in `{}`, what differs in `{}`",
            baseline.display(),
            tolerance.colour,
            tolerance.pixels * 100.0,
            actual_path.display(),
            diff_path.display()
        ),
        actual: actual_path,
        diff: Some(diff_path),
    })
}

fn within_share(differing: usize, total: usize, share: f64) -> bool {
    // Counts are exact in `f64`; over four billion pixels compare as that many.
    let count = |pixels: usize| f64::from(u32::try_from(pixels).unwrap_or(u32::MAX));
    count(differing) <= count(total) * share
}

fn write_png(path: &Path, picture: &Picture) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("`{}` cannot be made: {error}", parent.display()))?;
    }
    let bytes = png::encode(picture)?;
    fs::write(path, bytes)
        .map_err(|error| format!("`{}` cannot be written: {error}", path.display()))
}

fn remove_stale(actual: &Path, diff: &Path) {
    // A missing file is the usual case; one that cannot be removed only leaves
    // an old picture.
    let _ = fs::remove_file(actual);
    let _ = fs::remove_file(diff);
}

#[cfg(feature = "record")]
mod png {
    use alloc::{string::String, vec::Vec};

    use super::Picture;
    use crate::capture::Frame;

    pub(super) fn encode(picture: &Picture) -> Result<Vec<u8>, String> {
        Frame::new(picture.width, picture.height, picture.rgba.clone())
            .ok_or_else(|| "the picture's size does not fit its pixels".to_owned())?
            .to_png()
            .map_err(|error| error.to_string())
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Picture, String> {
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .map_err(|error| error.to_string())?
            .to_rgba8();
        Ok(Picture {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        })
    }
}

#[cfg(not(feature = "record"))]
mod png {
    use alloc::{string::String, vec::Vec};

    use super::Picture;

    const NEEDS_RECORD: &str = "reading and writing pictures needs feature `record`";

    pub(super) fn encode(_: &Picture) -> Result<Vec<u8>, String> {
        Err(NEEDS_RECORD.into())
    }

    pub(super) fn decode(_: &[u8]) -> Result<Picture, String> {
        Err(NEEDS_RECORD.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HeadlessPlugins, targets::Target};
    use bevy::{
        prelude::*,
        ui::{UiTransform, px},
    };

    const LAYOUT_FRAMES: u8 = 3;

    #[test]
    fn a_quarter_turn_swaps_the_width_and_height_of_the_crop() {
        let turned =
            Affine2::from_angle_translation(core::f32::consts::FRAC_PI_2, Vec2::new(100.0, 50.0));
        let (top_left, size) = bounds(turned, Vec2::new(40.0, 20.0));
        assert_eq!((top_left, size), (UVec2::new(90, 30), UVec2::new(20, 40)));
    }

    #[test]
    fn a_scale_grows_the_crop_about_the_centre() {
        let scaled =
            Affine2::from_scale_angle_translation(Vec2::new(2.0, 0.5), 0.0, Vec2::new(100.0, 50.0));
        let (top_left, size) = bounds(scaled, Vec2::new(40.0, 20.0));
        assert_eq!((top_left, size), (UVec2::new(60, 45), UVec2::new(80, 10)));
    }

    #[test]
    fn a_turn_of_forty_five_degrees_is_cropped_to_the_upright_bounding_box() {
        let turned =
            Affine2::from_angle_translation(core::f32::consts::FRAC_PI_4, Vec2::new(50.0, 50.0));
        let (top_left, size) = bounds(turned, Vec2::new(20.0, 20.0));
        // The diagonal is 28.28 long, so the box runs from 35.86 to 64.14.
        assert_eq!((top_left, size), (UVec2::new(35, 35), UVec2::new(30, 30)));
    }

    #[test]
    fn a_node_partly_left_of_the_picture_is_clipped_to_its_visible_part() {
        let (top_left, size) = bounds(
            Affine2::from_translation(Vec2::new(5.0, 20.0)),
            Vec2::new(30.0, 10.0),
        );
        assert_eq!((top_left, size), (UVec2::new(0, 15), UVec2::new(20, 10)));
    }

    #[test]
    fn the_crop_of_a_transformed_node_is_read_from_the_world() {
        let mut world = World::new();
        // A physical scale factor of 2 makes the computed size physical already.
        let node = world
            .spawn((
                ComputedNode {
                    size: Vec2::new(80.0, 40.0),
                    ..ComputedNode::default()
                },
                UiGlobalTransform::from(Affine2::from_angle_translation(
                    core::f32::consts::FRAC_PI_2,
                    Vec2::new(200.0, 100.0),
                )),
            ))
            .id();
        assert_eq!(
            node_rectangle(&world, node),
            Some((UVec2::new(180, 60), UVec2::new(40, 80)))
        );
    }

    fn laid_out_child_rectangle(
        transform: UiTransform,
        scale_factor: f32,
    ) -> Option<(UVec2, UVec2)> {
        let mut app = App::new();
        app.add_plugins(HeadlessPlugins::default().with_scale_factor(scale_factor));
        app.finish();
        app.cleanup();
        app.world_mut().spawn(Camera2d);
        let child = app
            .world_mut()
            .spawn(Node {
                position_type: PositionType::Absolute,
                left: px(20),
                top: px(10),
                width: px(40),
                height: px(30),
                ..Node::default()
            })
            .id();
        app.world_mut()
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(50),
                    top: px(50),
                    width: px(200),
                    height: px(100),
                    ..Node::default()
                },
                transform,
            ))
            .add_child(child);
        for _ in 0..LAYOUT_FRAMES {
            app.update();
        }
        node_rectangle(app.world(), child)
    }

    #[test]
    fn a_child_of_a_parent_turned_a_quarter_turn_is_cropped_with_width_and_height_swapped() {
        // The parent turns about its middle (150, 100); the child's middle,
        // (-60, -25) from there, lands at (25, -60) from it: (175, 40).
        let rectangle =
            laid_out_child_rectangle(UiTransform::from_rotation(Rot2::degrees(90.0)), 1.0);
        assert_eq!(rectangle, Some((UVec2::new(160, 20), UVec2::new(30, 40))));
    }

    #[test]
    fn a_child_of_a_scaled_parent_is_cropped_to_the_scaled_box_clipped_to_the_picture() {
        // Doubled about (150, 100) the child's middle moves to (30, 50) and
        // the box to 80 by 60, whose first ten columns lie left of the picture.
        let rectangle = laid_out_child_rectangle(UiTransform::from_scale(Vec2::splat(2.0)), 1.0);
        assert_eq!(rectangle, Some((UVec2::new(0, 20), UVec2::new(70, 60))));
    }

    #[test]
    fn at_a_physical_scale_factor_of_two_the_crop_is_in_physical_pixels() {
        // The same layout as the quarter turn above, twice as many pixels.
        let rectangle =
            laid_out_child_rectangle(UiTransform::from_rotation(Rot2::degrees(90.0)), 2.0);
        assert_eq!(rectangle, Some((UVec2::new(320, 40), UVec2::new(60, 80))));
    }

    #[test]
    fn a_picture_differs_only_past_the_colour_tolerance() {
        let a = Picture::filled(4, 4, [100, 100, 100, 255]);
        let near = Picture::filled(4, 4, [102, 99, 100, 255]);
        let far = Picture::filled(4, 4, [110, 100, 100, 255]);
        assert_eq!(a.differing(&near, 2), 0);
        assert_eq!(a.differing(&far, 2), 16);
    }

    #[test]
    fn a_crop_is_clipped_to_the_picture_and_empty_outside_it() {
        let a = Picture::filled(10, 10, [1, 2, 3, 255]);
        let part = a
            .crop(UVec2::new(8, 8), UVec2::new(5, 5))
            .unwrap_or_else(|| a.clone());
        assert_eq!((part.width, part.height), (2, 2));
        assert!(a.crop(UVec2::new(10, 0), UVec2::new(3, 3)).is_none());
    }

    #[test]
    fn a_diff_marks_the_pixels_that_differ_and_dims_the_rest() {
        let a = Picture::filled(2, 1, [90, 90, 90, 255]);
        let mut b = a.clone();
        if let Some(byte) = b.rgba.first_mut() {
            *byte = 200;
        }
        let diff = a.diff(&b, 0);
        assert_eq!(diff.pixel(0, 0), Some(&DIFF_COLOUR[..]));
        assert_eq!(diff.pixel(1, 0), Some(&[30, 30, 30, 255][..]));
    }

    #[test]
    fn a_share_of_pixels_is_allowed_to_differ() {
        assert!(within_share(0, 1000, 0.0));
        assert!(!within_share(1, 1000, 0.0));
        assert!(within_share(10, 1000, 0.0105));
        assert!(!within_share(11, 1000, 0.0105));
        assert!(within_share(1000, 1000, 1.0));
    }

    #[test]
    fn names_that_cannot_be_files_are_refused() {
        assert!(name_problem("home").is_none());
        for bad in ["", ".hidden", "a/b", "a\\b", "c:d"] {
            assert!(name_problem(bad).is_some(), "{bad:?}");
        }
    }

    #[test]
    fn a_tolerance_outside_zero_to_one_is_refused() {
        assert!(Tolerance::default().problem().is_none());
        assert!(Tolerance::default().with_pixels(1.5).problem().is_some());
        assert!(
            Tolerance::default()
                .with_pixels(f64::NAN)
                .problem()
                .is_some()
        );
    }

    #[test]
    fn a_scenarios_directory_is_its_name_made_safe() {
        assert_eq!(
            directory_name("Click the Pause [dark]!"),
            "click-the-pause-dark"
        );
        assert_eq!(directory_name("???"), "scenario");
    }

    fn scratch(test: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("rin-claims-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn scenarios_whose_names_make_the_same_directory_do_not_share_it() {
        let dir = scratch("collision");
        let folder = dir.join(directory_name("A B"));
        assert_eq!(directory_name("A B"), directory_name("A-B"));
        assert!(claim_directory(&folder, "A B").is_ok());
        assert!(claim_directory(&folder, "A B").is_ok());
        let refused = claim_directory(&folder, "A-B").err().unwrap_or_default();
        assert!(
            refused.contains("`A B`") && refused.contains("`A-B`"),
            "{refused}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn variants_whose_labels_make_the_same_directory_do_not_share_it() {
        let dir = scratch("variant-collision");
        let folder = dir.join(directory_name("Home [dark mode]"));
        assert!(claim_directory(&folder, "Home [dark mode]").is_ok());
        assert!(claim_directory(&folder, "Home [dark-mode]").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_of_baselines_from_before_owners_were_recorded_is_adopted() {
        let dir = scratch("legacy");
        let folder = dir.join("home");
        assert!(fs::create_dir_all(&folder).is_ok());
        assert!(fs::write(folder.join("home.png"), b"x").is_ok());
        assert!(claim_directory(&folder, "Home").is_ok());
        assert!(claim_directory(&folder, "HOME").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_screenshot_check_is_skipped_where_there_is_no_renderer() {
        use crate::{Outcome, Scenario};

        let report = Scenario::new("no renderer")
            .expect_check(Check::screenshot("home"))
            .expect_check(!Check::screenshot("home"))
            .run_headless();
        assert!(report.passed(), "{:?}", report.failures());
        assert_eq!(
            report
                .checks
                .iter()
                .map(|check| check.outcome.clone())
                .collect::<Vec<_>>(),
            vec![Outcome::Skipped, Outcome::Skipped]
        );
        assert_eq!(report.screenshots.len(), 0);
    }

    #[test]
    fn a_screenshot_check_is_read_from_a_file_and_written_back() {
        let text =
            r#"Screenshot(name: "home", of: Name("Bar"), tolerance: (colour: 4, pixels: 0.01))"#;
        let parsed = ron::from_str::<Check>(text);
        assert_eq!(
            parsed.ok(),
            Some(
                Check::screenshot_of("home", Target::name("Bar"))
                    .with_tolerance(Tolerance::default().with_colour(4).with_pixels(0.01))
            )
        );
        let bare = ron::from_str::<Check>(r#"Screenshot(name: "home")"#);
        assert_eq!(bare.ok(), Some(Check::screenshot("home")));
        let written = ron::to_string(&Check::screenshot("home")).unwrap_or_default();
        assert_eq!(written, r#"Screenshot(name:"home")"#);
        assert!(ron::from_str::<Check>(r#"Screenshot(name: "x", tolerance: (colur: 1))"#).is_err());
    }

    #[test]
    fn a_screenshot_with_a_bad_name_or_tolerance_fails_to_load() {
        let types = bevy::reflect::TypeRegistry::default();
        assert!(Check::screenshot("a/b").problem(&types).is_err());
        assert!(
            Check::screenshot("ok")
                .with_tolerance(Tolerance::default().with_pixels(2.0))
                .problem(&types)
                .is_err()
        );
        assert!(Check::screenshot("ok").problem(&types).is_ok());
    }
}

/// A screenshot check against a renderer that answers at once; no GPU needed.
#[cfg(all(test, feature = "record"))]
mod flow {
    use bevy::{
        app::{App, Last},
        ecs::{
            resource::Resource,
            schedule::IntoScheduleConfigs,
            system::{Res, ResMut},
        },
        prelude::{BackgroundColor, Camera2d, Color, Name, Node},
        ui::{PositionType, px},
    };

    use super::*;
    use crate::{Outcome, Scenario, player::ScenarioSystems};

    #[derive(Resource)]
    struct Paint([u8; 4]);

    fn fake_renderer(paint: Res<Paint>, mut pictures: ResMut<Pictures>) {
        if let Some(token) = pictures.begin_taking() {
            pictures.deliver(
                token,
                Ok(Picture::filled(
                    crate::DEFAULT_WINDOW_SIZE.0,
                    crate::DEFAULT_WINDOW_SIZE.1,
                    paint.0,
                )),
            );
        }
    }

    fn directory(test: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("rin-screenshots-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn run(dir: &Path, check: &Check, colour: [u8; 4], update: bool) -> crate::ScenarioReport {
        let scenario = Scenario::new("Home screen")
            .with_base_dir(dir)
            .setup(|world| {
                world.spawn(Camera2d);
                world.spawn((
                    Name::new("Bar"),
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(10),
                        top: px(20),
                        width: px(30),
                        height: px(40),
                        ..Node::default()
                    },
                    BackgroundColor(Color::WHITE),
                ));
            })
            .expect_check(check.clone());
        let mut app: App = scenario.headless_app();
        app.init_resource::<Pictures>()
            .insert_resource(ScreenshotSettings { update })
            .insert_resource(Paint(colour))
            .add_systems(Last, fake_renderer.after(ScenarioSystems::Trace));
        scenario.run_in(&mut app)
    }

    #[test]
    fn a_missing_baseline_fails_once_and_is_written() {
        let dir = directory("missing");
        let check = Check::screenshot("home");
        let first = run(&dir, &check, [10, 20, 30, 255], false);
        assert!(!first.passed());
        assert!(dir.join("home-screen/home.png").exists());
        assert!(matches!(
            first.screenshots.first().map(|record| &record.result),
            Some(ScreenshotResult::Created)
        ));
        let second = run(&dir, &check, [10, 20, 30, 255], false);
        assert!(second.passed(), "{:?}", second.failures());
        assert!(matches!(
            second.screenshots.first().map(|record| &record.result),
            Some(ScreenshotResult::Matched)
        ));
    }

    #[test]
    fn a_changed_colour_fails_with_a_picture_and_a_diff() {
        let dir = directory("changed");
        let check = Check::screenshot("home");
        run(&dir, &check, [10, 20, 30, 255], false);
        let report = run(&dir, &check, [10, 20, 90, 255], false);
        assert!(!report.passed());
        let folder = dir.join("home-screen");
        assert!(folder.join(".diagnostics/home.actual.png").exists());
        assert!(folder.join(".diagnostics/home.diff.png").exists());
        assert!(matches!(
            report.screenshots.first().map(|record| &record.result),
            Some(ScreenshotResult::Mismatch { diff: Some(_), .. })
        ));
        let loose = check.with_tolerance(Tolerance::default().with_colour(100));
        let accepted = run(&dir, &loose, [10, 20, 90, 255], false);
        assert!(accepted.passed(), "{:?}", accepted.failures());
        assert!(!folder.join(".diagnostics/home.actual.png").exists());
    }

    fn baseline_of(dir: &Path, name: &str) -> Vec<u8> {
        fs::read(dir.join(format!("home-screen/{name}.png"))).unwrap_or_default()
    }

    #[test]
    fn diagnostics_never_overwrite_or_delete_a_baseline_with_a_colliding_name() {
        let dir = directory("colliding");
        let home = Check::screenshot("home");
        let other = Check::screenshot("home.actual");
        let diff = Check::screenshot("home.diff");
        for check in [&home, &other, &diff] {
            run(&dir, check, [10, 20, 30, 255], false);
        }
        // Give each baseline its own color so a clobbered one shows.
        run(&dir, &other, [1, 2, 3, 255], true);
        run(&dir, &diff, [4, 5, 6, 255], true);
        let (kept_other, kept_diff) = (
            baseline_of(&dir, "home.actual"),
            baseline_of(&dir, "home.diff"),
        );
        assert!(!kept_other.is_empty() && !kept_diff.is_empty());

        let mismatch = run(&dir, &home, [10, 20, 90, 255], false);
        assert!(!mismatch.passed());
        assert_eq!(baseline_of(&dir, "home.actual"), kept_other);
        assert_eq!(baseline_of(&dir, "home.diff"), kept_diff);

        let matched = run(&dir, &home, [10, 20, 30, 255], false);
        assert!(matched.passed(), "{:?}", matched.failures());
        assert_eq!(baseline_of(&dir, "home.actual"), kept_other);
        assert_eq!(baseline_of(&dir, "home.diff"), kept_diff);
        assert!(
            !dir.join("home-screen/.diagnostics/home.actual.png")
                .exists()
        );

        run(&dir, &home, [10, 20, 90, 255], false);
        let updated = run(&dir, &home, [10, 20, 90, 255], true);
        assert!(updated.passed(), "{:?}", updated.failures());
        assert_eq!(baseline_of(&dir, "home.actual"), kept_other);
        assert_eq!(baseline_of(&dir, "home.diff"), kept_diff);

        let still = run(&dir, &other, [1, 2, 3, 255], false);
        assert!(still.passed(), "{:?}", still.failures());
    }

    #[test]
    fn pictures_an_older_version_left_beside_the_baselines_are_not_touched() {
        let dir = directory("legacy");
        let check = Check::screenshot("home");
        run(&dir, &check, [10, 20, 30, 255], false);
        let legacy = dir.join("home-screen/home.actual.png");
        fs::write(&legacy, b"old").unwrap_or_default();
        run(&dir, &check, [10, 20, 90, 255], false);
        run(&dir, &check, [10, 20, 30, 255], false);
        run(&dir, &check, [10, 20, 90, 255], true);
        assert_eq!(fs::read(&legacy).unwrap_or_default(), b"old");
    }

    #[test]
    fn updating_rewrites_the_baseline_and_passes() {
        let dir = directory("update");
        let check = Check::screenshot("home");
        run(&dir, &check, [10, 20, 30, 255], false);
        let updated = run(&dir, &check, [200, 20, 30, 255], true);
        assert!(updated.passed(), "{:?}", updated.failures());
        let matched = run(&dir, &check, [200, 20, 30, 255], false);
        assert!(matched.passed(), "{:?}", matched.failures());
    }

    #[test]
    fn a_target_gives_the_screenshot_the_size_of_its_node() {
        let dir = directory("target");
        let check = Check::screenshot_of("bar", Target::name("Bar"));
        run(&dir, &check, [10, 20, 30, 255], false);
        let bytes = fs::read(dir.join("home-screen/bar.png")).unwrap_or_default();
        let baseline = png::decode(&bytes).unwrap_or_else(|_| Picture::filled(0, 0, [0; 4]));
        assert_eq!((baseline.width, baseline.height), (30, 40));
        let missing = Check::screenshot_of("none", Target::name("Nobody"));
        let report = run(&dir, &missing, [10, 20, 30, 255], false);
        assert!(
            report
                .checks
                .iter()
                .any(|check| matches!(check.outcome, Outcome::Fail(_)))
        );
    }
}
