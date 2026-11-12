//! Recording scenarios offscreen, for people to review (feature `record`).
//!
//! The real renderer runs with every camera pointed at an [`Image`] of the
//! scenario's window size, and frames are read back through `Screenshot`. No
//! window or display is needed; on a machine without a GPU, set
//! `WGPU_BACKEND=vulkan` and install lavapipe.
//!
//! [`RecorderPlugins`] is the render set the app's own plugins go on top of.
//! [`Recorder`] builds one app per scenario, plays it, and writes PNGs, a
//! filmstrip, a GIF and an `index.html`.

use alloc::{collections::BTreeMap, sync::Arc};
use core::fmt;
use std::{fs, io, path::Path, path::PathBuf};

use bevy::{
    app::{App, First, Last, Plugin, PluginGroup, PluginGroupBuilder, PostUpdate, Startup},
    asset::{Assets, Handle, RenderAssetUsages},
    camera::{
        Camera, CameraUpdateSystems, ImageRenderTarget, NormalizedRenderTarget, RenderTarget,
        visibility::{Visibility, VisibilitySystems},
    },
    core_pipeline::CorePipelinePlugin,
    ecs::{
        component::Component,
        entity::Entity,
        observer::On,
        query::{Added, With},
        resource::Resource,
        schedule::{IntoScheduleConfigs, SystemSet},
        system::{Commands, ParamSet, Query, Res, ResMut},
        world::World,
    },
    image::{Image, ImagePlugin},
    picking::input::PointerInputSettings,
    render::{
        RenderPlugin,
        render_resource::TextureFormat,
        view::window::screenshot::{Screenshot, ScreenshotCaptured},
    },
    sprite_render::SpriteRenderPlugin,
    ui::{DefaultUiCamera, IsDefaultUiCamera, UiPlugin, UiSystems},
    ui_render::UiRenderPlugin,
    window::{PrimaryWindow, Window, WindowRef},
};

use crate::{
    annotation::{Disabled, TagFilter},
    capture::{CaptureError, Frame},
    check::{
        ScreenshotSettings,
        screenshot::{Picture, Pictures},
        suggest::NoiseFilter,
    },
    headless::{DEFAULT_SCALE_FACTOR, DEFAULT_WINDOW_SIZE, HeadlessPlugins, RenderCameraSupport},
    index,
    input::SyntheticInput,
    overlay::{CursorOverlayPlugin, OverlayRoot, TargetDescriptions, current_step_text},
    player::{Outcome, ScenarioPlayer, ScenarioPlayerPlugin, ScenarioReport, ScenarioSystems},
    scenario::Scenario,
    trace::ScenarioTrace,
    variant::Variant,
};

/// Every this many frames a picture is taken, unless configured otherwise.
pub const DEFAULT_EVERY: u32 = 10;
/// Updates run before the scenario starts, however fast the app settles, so
/// layout, the glyph atlas and the render pipelines are in place.
pub const MIN_SETTLE_FRAMES: u32 = 4;
/// Updates after which an app that has still not settled is an error.
pub const MAX_SETTLE_FRAMES: u32 = 1200;
/// Updates after the scenario ends that pictures still on their way to the
/// CPU are waited for.
pub const MAX_DRAIN_FRAMES: u32 = 240;
/// Upper bound on a scenario's length in frames, so a script that never ends
/// is an error and not a hang.
pub const MAX_SCENARIO_FRAMES: u32 = 1_000_000;
/// The group of a scenario added without one.
pub const DEFAULT_GROUP: &str = "Scenarios";

/// The image the app's cameras render to, and where captured pictures wait.
#[derive(Resource, Debug, Clone)]
pub struct RecorderTarget(pub Handle<Image>);

/// What the recorder knows about a picture when it asks for it.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct CaptureMeta {
    /// The player frame the picture shows; `0` is before the first step.
    pub frame: u32,
    /// The steps that began on this frame, by index into the script.
    pub steps: Vec<usize>,
    /// The step, in words, as the caption strip shows it.
    pub step_text: String,
    /// The checks so far, in words.
    pub checks_text: String,
    /// The scenario's own caption at this frame.
    pub caption: Option<String>,
    /// Whether a step or check had failed by this frame.
    pub failed: bool,
}

/// A picture with what was known when it was taken.
#[derive(Debug, Clone)]
pub struct Capture {
    /// What was going on.
    pub meta: CaptureMeta,
    /// The pixels.
    pub frame: Frame,
}

/// Pictures asked for, and pictures that have come back, by frame.
#[derive(Resource, Debug, Default)]
pub struct Captures {
    /// Take a picture every this many frames; `0` means only at step
    /// boundaries and the end.
    pub every: u32,
    /// Whether the capture system is asking for pictures.
    pub enabled: bool,
    /// How many pictures have been asked for.
    requested: u32,
    /// The last frame a picture was asked for, so none is asked for twice.
    last_requested: Option<u32>,
    /// Pictures that have come back, ordered by frame.
    done: BTreeMap<u32, Capture>,
}

impl Captures {
    /// How many pictures were asked for and have not come back yet.
    #[must_use]
    pub fn outstanding(&self) -> u32 {
        self.requested
            .saturating_sub(u32::try_from(self.done.len()).unwrap_or(u32::MAX))
    }

    /// The pictures that have come back, in frame order.
    pub fn captures(&self) -> impl Iterator<Item = &Capture> {
        self.done.values()
    }
}

/// System sets of the recorder, all in `PostUpdate` or `Last`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecorderSystems {
    /// Points cameras at the recorder's image, in `PostUpdate`.
    Retarget,
    /// Asks for pictures of the frame just played, in `Last`.
    Capture,
}

/// The render set the app's own plugins go on top of.
///
/// It is a [`HeadlessPlugins`] app with the real renderer, `bevy_ui_render`
/// (so the UI and the cursor are in the picture), the scenario player and the
/// cursor overlay.
///
/// Do not add [`ScenarioPlayerPlugin`] or [`CursorOverlayPlugin`] again. The
/// primary window stays virtual: it carries the cursor position and the size,
/// and every camera that would render to it renders to an image instead, which
/// `SyntheticInput` aims its pointer at.
#[derive(Debug, Clone, Copy)]
pub struct RecorderPlugins {
    /// Size of the virtual primary window, in logical pixels.
    window_size: (u32, u32),
    /// Physical pixels per logical pixel; the image has that many times the
    /// pixels of the window size.
    scale_factor: f32,
}

impl Default for RecorderPlugins {
    fn default() -> Self {
        Self {
            window_size: DEFAULT_WINDOW_SIZE,
            scale_factor: DEFAULT_SCALE_FACTOR,
        }
    }
}

impl RecorderPlugins {
    /// Sets the size of the window, in logical pixels. The image has that
    /// size times the scale factor, in physical pixels.
    #[must_use]
    pub const fn with_window_size(mut self, width: u32, height: u32) -> Self {
        self.window_size = (width, height);
        self
    }

    /// Sets the scale factor: physical pixels per logical pixel, as of
    /// [`HeadlessPlugins::with_scale_factor`]. The UI lays out for the logical
    /// size and the pictures have that many times the pixels. A value that is
    /// not finite and above zero is ignored.
    #[must_use]
    pub const fn with_scale_factor(mut self, scale_factor: f32) -> Self {
        if scale_factor.is_finite() && scale_factor > 0.0 {
            self.scale_factor = scale_factor;
        }
        self
    }
}

impl PluginGroup for RecorderPlugins {
    fn build(self) -> PluginGroupBuilder {
        let (width, height) = self.window_size;
        HeadlessPlugins::default()
            .with_window_size(width, height)
            .with_scale_factor(self.scale_factor)
            .build()
            // The render plugin brings the render crate's camera plugin.
            .disable::<RenderCameraSupport>()
            .add_before::<ImagePlugin>(RenderPlugin {
                // Otherwise the first frames are drawn without their pipelines
                // and frame 0 would differ from run to run.
                synchronous_pipeline_compilation: true,
                ..RenderPlugin::default()
            })
            .add_after::<ImagePlugin>(CorePipelinePlugin)
            // `bevy_ui_render` borrows the sprite renderer's image bookkeeping.
            .add_after::<UiPlugin>(SpriteRenderPlugin)
            .add_after::<SpriteRenderPlugin>(UiRenderPlugin)
            .add(ScenarioPlayerPlugin)
            .add(CursorOverlayPlugin)
            .add(RecorderSupport)
    }
}

/// Aims cameras and pointer at the recorder's image and takes pictures.
#[derive(Debug, Default, Clone, Copy)]
struct RecorderSupport;

impl Plugin for RecorderSupport {
    fn build(&self, app: &mut App) {
        app.init_resource::<Captures>()
            .init_resource::<Pictures>()
            .init_resource::<ScreenshotSettings>()
            // The synthetic pointer writes `PointerInput` itself; Bevy's own
            // mouse picking would add a second one aimed at the window, which
            // nothing renders to, and the last one wins.
            .insert_resource(PointerInputSettings {
                is_mouse_enabled: false,
                is_touch_enabled: false,
            })
            .add_observer(store_capture)
            .add_observer(store_check_picture)
            .add_systems(First, show_overlay_again)
            .add_systems(Startup, create_target)
            .add_systems(
                PostUpdate,
                // Every set that reads a camera's target is named on its own:
                // Bevy chains them only weakly, so being before the first of
                // them would no longer say anything about the others.
                hide_overlay_for_picture
                    .before(VisibilitySystems::VisibilityPropagate),
            )
            .add_systems(
                PostUpdate,
                retarget_cameras
                    .in_set(RecorderSystems::Retarget)
                    .before(CameraUpdateSystems)
                    .before(UiSystems::Prepare)
                    .before(UiSystems::Propagate)
                    .before(UiSystems::Layout),
            )
            .add_systems(
                Last,
                // After the trace point, so a point's frame gets its picture.
                request_frames
                    .in_set(RecorderSystems::Capture)
                    .after(ScenarioSystems::Trace),
            )
            .add_systems(
                Last,
                request_check_picture.after(RecorderSystems::Capture),
            );
    }
}

/// Marks the screenshot taken for a screenshot check, with the request it
/// answers.
#[derive(Component, Debug, Clone, Copy)]
struct CheckPicture(u32);

/// Takes the picture a screenshot check wants, of the frame just played.
fn request_check_picture(
    mut commands: Commands,
    mut pictures: ResMut<Pictures>,
    target: Option<Res<RecorderTarget>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    already: Query<Entity, (With<CaptureMeta>, Added<Screenshot>)>,
) {
    let Some(target) = target else {
        return;
    };
    let Some(token) = pictures.begin_taking() else {
        return;
    };
    // Bevy keeps one screenshot of a target a frame: a second would replace or
    // lose the first, so the recording's picture of this frame answers both.
    if let Some(entity) = already.iter().next() {
        commands.entity(entity).insert(CheckPicture(token));
        return;
    }
    let scale_factor = windows
        .iter()
        .next()
        .map_or(DEFAULT_SCALE_FACTOR, |window| {
            window.resolution.scale_factor()
        });
    commands.spawn((screenshot(&target.0, scale_factor), CheckPicture(token)));
}

/// Hands a screenshot check the picture that came back for it.
fn store_check_picture(
    event: On<ScreenshotCaptured>,
    requests: Query<&CheckPicture>,
    mut pictures: ResMut<Pictures>,
) {
    let Ok(request) = requests.get(event.entity) else {
        return;
    };
    let picture = Frame::from_image(&event.image)
        .map(|frame| Picture {
            width: frame.width(),
            height: frame.height(),
            rgba: frame.rgba().to_vec(),
        })
        .map_err(|error| format!("the picture could not be read: {error}"));
    pictures.deliver(request.0, picture);
}

/// Hides the cursor, caption and the other overlay from the frame a screenshot
/// check is about to take, before visibility is worked out, so a baseline
/// holds the app and not the tooling.
fn hide_overlay_for_picture(
    mut pictures: ResMut<Pictures>,
    mut roots: Query<(Entity, &mut Visibility), With<OverlayRoot>>,
) {
    if !pictures.is_wanted() {
        return;
    }
    let mut hidden = Vec::new();
    for (entity, mut visibility) in &mut roots {
        hidden.push((entity, *visibility));
        *visibility = Visibility::Hidden;
    }
    pictures.hide(hidden);
}

/// Puts the overlay back after a picture was taken without it.
fn show_overlay_again(mut pictures: ResMut<Pictures>, mut roots: Query<&mut Visibility>) {
    for (entity, before) in pictures.take_hidden() {
        if let Ok(mut visibility) = roots.get_mut(entity) {
            *visibility = before;
        }
    }
}

/// Creates the image and aims the synthetic pointer at it.
fn create_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut input: ResMut<SyntheticInput>,
) {
    let (width, height, scale_factor) = windows.iter().next().map_or(
        (
            DEFAULT_WINDOW_SIZE.0,
            DEFAULT_WINDOW_SIZE.1,
            DEFAULT_SCALE_FACTOR,
        ),
        |window| {
            (
                window.resolution.physical_width(),
                window.resolution.physical_height(),
                window.resolution.scale_factor(),
            )
        },
    );
    let mut image = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    // Read back through the screenshot's own copy, but keep the data on the
    // CPU side so the handle is also a normal image.
    image.asset_usage = RenderAssetUsages::default();
    let handle = images.add(image);
    input.set_target(Some(NormalizedRenderTarget::Image(image_target(
        &handle,
        scale_factor,
    ))));
    commands.insert_resource(RecorderTarget(handle));
}

/// The image as a render target at the window's scale factor. The cameras'
/// target and the synthetic pointer's must be equal, scale factor included, or
/// the pointer's position is not on any camera's target.
fn image_target(handle: &Handle<Image>, scale_factor: f32) -> ImageRenderTarget {
    ImageRenderTarget {
        handle: handle.clone(),
        scale_factor,
    }
}

/// Points every camera that renders to the primary window at the image.
///
/// A camera that renders to an image is never Bevy's default UI camera, so the
/// one that would have been gets `IsDefaultUiCamera`; otherwise the app's UI
/// and the overlay would vanish from the picture.
fn retarget_cameras(
    mut commands: Commands,
    target: Option<Res<RecorderTarget>>,
    mut cameras: ParamSet<(
        DefaultUiCamera,
        Query<(Entity, &mut RenderTarget), With<Camera>>,
    )>,
    marked: Query<(), With<IsDefaultUiCamera>>,
    primary: Query<(Entity, &Window), With<PrimaryWindow>>,
) {
    let Some(target) = target else {
        return;
    };
    let default = cameras.p0().get();
    let primary = primary.iter().next();
    let scale_factor = primary.map_or(DEFAULT_SCALE_FACTOR, |(_, window)| {
        window.resolution.scale_factor()
    });
    let primary = primary.map(|(entity, _)| entity);
    for (entity, mut render_target) in &mut cameras.p1() {
        let to_window = match &*render_target {
            RenderTarget::Window(WindowRef::Primary) => true,
            RenderTarget::Window(WindowRef::Entity(window)) => Some(*window) == primary,
            _ => false,
        };
        if !to_window {
            continue;
        }
        *render_target = RenderTarget::Image(image_target(&target.0, scale_factor));
        if default == Some(entity) && marked.is_empty() {
            commands.entity(entity).insert(IsDefaultUiCamera);
        }
    }
}

/// Asks for a picture of the frame just played when it is one worth having:
/// a step began, the trace took a point, it is every `every`th frame, or the
/// scenario just finished.
fn request_frames(
    mut commands: Commands,
    mut captures: ResMut<Captures>,
    report: Res<ScenarioReport>,
    player: Res<ScenarioPlayer>,
    descriptions: Res<TargetDescriptions>,
    theme: Res<crate::overlay::OverlayTheme>,
    target: Option<Res<RecorderTarget>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let frame = report.frames;
    if !captures.enabled || frame == 0 || captures.last_requested == Some(frame) {
        return;
    }
    let Some(target) = target else {
        return;
    };
    let steps: Vec<usize> = report
        .steps
        .iter()
        .enumerate()
        .filter(|(_, record)| record.frame == frame && !matches!(record.outcome, Outcome::Skipped))
        .map(|(index, _)| index)
        .collect();
    let on_interval = captures.every != 0 && frame.is_multiple_of(captures.every);
    let traced = player
        .trace()
        .and_then(|trace| trace.last())
        .is_some_and(|point| point.frame == frame);
    if steps.is_empty() && !on_interval && !traced && !report.finished {
        return;
    }
    let meta = CaptureMeta {
        frame,
        steps,
        step_text: current_step_text(&report, &descriptions).unwrap_or_default(),
        checks_text: crate::overlay::checks_text(&report.checks, &theme),
        caption: report.caption.clone(),
        failed: has_failed(&report),
    };
    captures.last_requested = Some(frame);
    captures.requested += 1;
    let scale_factor = windows
        .iter()
        .next()
        .map_or(DEFAULT_SCALE_FACTOR, |window| {
            window.resolution.scale_factor()
        });
    commands.spawn((screenshot(&target.0, scale_factor), meta));
}

/// A screenshot of the recorder's image. It names the image with the scale
/// factor the cameras render to it at: Bevy keys the screenshot's output by
/// the whole render target, so one at another scale factor would be a
/// different target, the cameras would not draw into it, and the picture would
/// be black.
fn screenshot(handle: &Handle<Image>, scale_factor: f32) -> Screenshot {
    Screenshot(RenderTarget::Image(image_target(handle, scale_factor)))
}

/// Whether any step or check has failed so far.
fn has_failed(report: &ScenarioReport) -> bool {
    report
        .steps
        .iter()
        .map(|record| &record.outcome)
        .chain(report.checks.iter().map(|record| &record.outcome))
        .any(|outcome| matches!(outcome, Outcome::Fail(_)))
}

/// Keeps a picture that came back from the GPU.
fn store_capture(
    event: On<ScreenshotCaptured>,
    metas: Query<&CaptureMeta>,
    mut captures: ResMut<Captures>,
) {
    let Some(meta) = metas.get(event.entity).ok().cloned() else {
        return;
    };
    match Frame::from_image(&event.image) {
        Ok(frame) => {
            captures.done.insert(meta.frame, Capture { meta, frame });
        }
        Err(error) => {
            bevy::log::error!("a recorded frame could not be read: {error}");
        }
    }
}

/// Asks for a picture of the app as it is now, outside the player's frames.
/// The picture comes back after the next update.
pub fn request_capture(world: &mut World, meta: CaptureMeta) {
    let Some(target) = world.get_resource::<RecorderTarget>().cloned() else {
        return;
    };
    if let Some(mut captures) = world.get_resource_mut::<Captures>() {
        captures.requested += 1;
    }
    let scale_factor = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .iter(world)
        .next()
        .map_or(DEFAULT_SCALE_FACTOR, |window| {
            window.resolution.scale_factor()
        });
    world.spawn((screenshot(&target.0, scale_factor), meta));
}

/// Why recording failed. A scenario whose checks fail is not an error: it is
/// reported in the [`Recording`].
#[derive(Debug)]
#[non_exhaustive]
pub enum RecordError {
    /// A file or directory could not be written.
    Io(PathBuf, io::Error),
    /// A picture could not be read or encoded.
    Capture(CaptureError),
    /// The app's settle hook never said it had settled.
    NeverSettled {
        /// The scenario.
        scenario: String,
    },
    /// The scenario never finished.
    NeverFinished {
        /// The scenario.
        scenario: String,
    },
    /// Pictures never came back from the GPU: there is probably no usable
    /// one (try `WGPU_BACKEND=vulkan` with lavapipe installed).
    NoPictures {
        /// The scenario.
        scenario: String,
        /// How many pictures were asked for and never arrived.
        missing: u32,
    },
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(path, error) => write!(f, "{}: {error}", path.display()),
            Self::Capture(error) => write!(f, "{error}"),
            Self::NeverSettled { scenario } => {
                write!(f, "`{scenario}`: the app never settled")
            }
            Self::NeverFinished { scenario } => {
                write!(f, "`{scenario}`: the scenario never finished")
            }
            Self::NoPictures { scenario, missing } => write!(
                f,
                "`{scenario}`: {missing} picture(s) never came back from the GPU; is there a \
                 (software) Vulkan driver and is WGPU_BACKEND set?"
            ),
        }
    }
}

impl core::error::Error for RecordError {}

impl From<CaptureError> for RecordError {
    fn from(error: CaptureError) -> Self {
        Self::Capture(error)
    }
}

/// What was recorded for one scenario.
#[derive(Debug, Clone)]
pub struct ScenarioRecording {
    /// The scenario's name.
    pub name: String,
    /// The group it is listed under.
    pub group: String,
    /// Its directory under the output directory, also its anchor in the index.
    pub slug: String,
    /// How it ended.
    pub report: ScenarioReport,
    /// Whether the scenario does not run in its variant and was skipped rather
    /// than played (see [`Variant::skips`]).
    pub skipped: bool,
    /// The scenario's tags (see [`Scenario::tags`]).
    pub tags: Vec<String>,
    /// Why the scenario was not played, when it is skipped or fixme (see
    /// [`Scenario::skip`], [`Scenario::fixme`]).
    pub disabled: Option<Disabled>,
    /// Why the scenario is expected to fail, when it is (see
    /// [`Scenario::should_fail`]).
    pub should_fail: Option<String>,
    /// How many pictures were written.
    pub pictures: usize,
    /// The world around each step, which `trace.html` in the scenario's
    /// directory shows.
    pub trace: Option<ScenarioTrace>,
}

impl ScenarioRecording {
    /// Whether the scenario finished with every step and check passing.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.report.passed() != self.should_fail.is_some()
    }

    /// Whether the scenario failed and was expected to
    /// ([`Scenario::should_fail`]), which counts as a pass.
    #[must_use]
    pub fn failed_as_expected(&self) -> bool {
        self.should_fail.is_some() && !self.report.passed()
    }

    /// Whether the scenario does not run in its variant and was skipped rather
    /// than played. A skipped scenario is nothing to fail over.
    #[must_use]
    pub const fn skipped(&self) -> bool {
        self.skipped
    }

    /// Whether the scenario is skipped or fixme and was not played, whatever
    /// the variant.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        self.disabled.is_some()
    }

    /// Whether the scenario ran and failed.
    #[must_use]
    pub fn failed(&self) -> bool {
        !self.passed() && !self.skipped && self.disabled.is_none()
    }
}

/// Everything a [`Recorder::record`] run wrote.
#[derive(Debug, Clone)]
pub struct Recording {
    /// The output directory.
    pub directory: PathBuf,
    /// The index page, `index.html` in the output directory.
    pub index: PathBuf,
    /// One entry per scenario, in the order they were added.
    pub scenarios: Vec<ScenarioRecording>,
}

impl Recording {
    /// Whether any scenario failed, for the calling binary to exit non-zero. A
    /// scenario that was skipped is nothing to fail over.
    #[must_use]
    pub fn any_failed(&self) -> bool {
        self.scenarios.iter().any(ScenarioRecording::failed)
    }
}

/// Everything a [`Recorder::check`] run played: outcomes only, nothing written.
#[derive(Debug, Clone)]
pub struct Checked {
    /// One entry per scenario, in the order they were added; no pictures, no
    /// trace.
    pub scenarios: Vec<ScenarioRecording>,
}

impl Checked {
    /// Whether any scenario failed, for the calling binary to exit non-zero. A
    /// scenario that was skipped is nothing to fail over.
    #[must_use]
    pub fn any_failed(&self) -> bool {
        self.scenarios.iter().any(ScenarioRecording::failed)
    }
}

/// What playing one scenario produced: its report, its pictures and, as the
/// recorder always asks for one, its trace.
type PlayedScenario = (ScenarioReport, Vec<Capture>, Option<ScenarioTrace>);
/// How the app says its world has settled.
type SettleFn = dyn Fn(&mut World) -> bool + Send + Sync;
/// How the app adds its own plugins to the recorder's render set.
type BuildFn = dyn Fn(&mut App) + Send + Sync;

/// One scenario and where it is listed.
struct Entry {
    /// The group in the index.
    group: String,
    /// The scenario.
    scenario: Scenario,
}

/// Records scenarios offscreen: PNGs at step boundaries and every `n`th frame,
/// a filmstrip and a GIF per scenario, and an `index.html` over all of them.
///
/// There is no golden-image comparison; the point is for a person to look.
/// Whether any scenario failed is in [`Recording::any_failed`].
pub struct Recorder {
    /// Adds the app's own plugins, resources and registry.
    build: Arc<BuildFn>,
    /// The scenarios, in order.
    entries: Vec<Entry>,
    /// Says when the app's world has settled.
    settle: Option<Arc<SettleFn>>,
    /// A picture every this many frames.
    every: u32,
    /// Which reflected types the traces' snapshots leave out.
    noise: NoiseFilter,
    /// The environments every scenario is recorded in, each its own index
    /// entry; the default one (the scenario as written) unless set.
    variants: Vec<Variant>,
    /// Which scenarios are recorded, by tag.
    tags: TagFilter,
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recorder")
            .field("scenarios", &self.entries.len())
            .field("every", &self.every)
            .finish_non_exhaustive()
    }
}

impl Recorder {
    /// A recorder for an app. `build` runs on a fresh app that already has
    /// [`RecorderPlugins`]: it adds the app's own plugins and resources, and
    /// anything its scenarios' setups and checks need (for example
    /// [`crate::script::Registry::install`]).
    pub fn new(build: impl Fn(&mut App) + Send + Sync + 'static) -> Self {
        Self {
            build: Arc::new(build),
            entries: Vec::new(),
            settle: None,
            every: DEFAULT_EVERY,
            noise: NoiseFilter::default(),
            variants: vec![Variant::default()],
            tags: TagFilter::default(),
        }
    }

    /// Adds a scenario to the group [`DEFAULT_GROUP`].
    #[must_use]
    pub fn scenario(self, scenario: Scenario) -> Self {
        self.grouped(DEFAULT_GROUP, scenario)
    }

    /// Adds a scenario under a group of the index.
    #[must_use]
    pub fn grouped(mut self, group: impl Into<String>, scenario: Scenario) -> Self {
        self.entries.push(Entry {
            group: group.into(),
            scenario,
        });
        self
    }

    /// Records every scenario once per variant, each its own entry in the
    /// index named `scenario [variant]`, instead of only as written. List
    /// [`Variant::default`] too to keep the scenario as written. A variant a
    /// scenario does not name (see [`Variant::skips`]) gets a skipped entry,
    /// like the `.ron` test runner; a variant that it cannot run in (a step
    /// aimed at a screen position, in a variant that changes the window) is
    /// recorded as a failed entry saying so. An empty list records nothing.
    ///
    /// Name the variants differently ([`Variant::new`]): two with one name get
    /// separate folders, but their index entries carry the same title.
    #[must_use]
    pub fn variants(mut self, variants: impl IntoIterator<Item = Variant>) -> Self {
        self.variants = variants.into_iter().collect();
        self
    }

    /// Records only the scenarios whose tags `tags` lets through (see
    /// [`TagFilter`]); by default all of them. A scenario with `skip` or
    /// `fixme` is listed in the index with its reason and not played.
    #[must_use]
    pub fn with_tag_filter(mut self, tags: TagFilter) -> Self {
        self.tags = tags;
        self
    }

    /// Takes a picture every `frames` frames as well as at step boundaries;
    /// `0` turns the interval off.
    #[must_use]
    pub const fn every(mut self, frames: u32) -> Self {
        self.every = frames;
        self
    }

    /// Sets which reflected types the per-step snapshots in `trace.html` leave
    /// out; by default Bevy's own and this crate's.
    #[must_use]
    pub fn with_noise_filter(mut self, noise: NoiseFilter) -> Self {
        self.noise = noise;
        self
    }

    /// Lets the app say when its world has settled (for example, terrain
    /// chunks have loaded), before step 1, so frame 0 is the same on every
    /// run. It is called once per update, after the setup and the view, and
    /// returns `true` when settled. At least [`MIN_SETTLE_FRAMES`] updates run
    /// whatever it says.
    #[must_use]
    pub fn settle(mut self, settled: impl Fn(&mut World) -> bool + Send + Sync + 'static) -> Self {
        self.settle = Some(Arc::new(settled));
        self
    }

    /// Plays every scenario and writes the result under `directory`.
    ///
    /// # Errors
    ///
    /// Fails when a file cannot be written, a picture never comes back, or an
    /// app never settles or never finishes. A scenario that merely fails its
    /// checks is not an error.
    pub fn record(&self, directory: impl AsRef<Path>) -> Result<Recording, RecordError> {
        let directory = directory.as_ref();
        create_dir(directory)?;
        let scenarios = self.run_all(true, |recording, pictures| {
            index::write_scenario(&directory.join(&recording.slug), pictures, recording)
        })?;
        let index = directory.join("index.html");
        write_file(&index, index::index_html(&scenarios).as_bytes())?;
        Ok(Recording {
            directory: directory.to_path_buf(),
            index,
            scenarios,
        })
    }

    /// Plays every scenario with the real renderer, as [`Recorder::record`]
    /// does, but takes no pictures, builds no traces and writes no files: for
    /// a CI job that only needs the checks. Outcomes are the same as in a
    /// recording, so [`Checked::any_failed`] says whether to exit non-zero.
    ///
    /// Screenshot checks (see [`crate::Check::screenshot`]) still take their
    /// pictures and read or write their baselines, which are test data and not
    /// a recording. The scenarios' [`ScenarioRecording::pictures`] is `0` and
    /// their [`ScenarioRecording::trace`] is `None`.
    ///
    /// # Errors
    ///
    /// Fails when an app never settles or never finishes, or a screenshot
    /// check's picture never comes back. A scenario that merely fails its
    /// checks is not an error.
    pub fn check(&self) -> Result<Checked, RecordError> {
        let scenarios = self.run_all(false, |_, _| Ok(()))?;
        Ok(Checked { scenarios })
    }

    /// Plays every scenario in every variant. `capture` says whether pictures
    /// and traces are taken; `write` is handed each played scenario with its
    /// pictures.
    fn run_all(
        &self,
        capture: bool,
        mut write: impl FnMut(&ScenarioRecording, &[Capture]) -> Result<(), RecordError>,
    ) -> Result<Vec<ScenarioRecording>, RecordError> {
        let mut used = Vec::new();
        let mut scenarios = Vec::new();
        for entry in &self.entries {
            if !self.tags.matches(entry.scenario.tag_names()) {
                continue;
            }
            let disabled = entry.scenario.disabled();
            for variant in &self.variants {
                let scenario = variant.varied(&entry.scenario);
                let slug = unique_slug(scenario.name(), &mut used);
                // A scenario that does not name this variant is skipped, not
                // failed, and nothing is played or written for it; the `.ron`
                // test runner does the same.
                let skipped = variant.skips(&entry.scenario);
                let (report, pictures, trace) = if skipped || disabled.is_some() {
                    (
                        ScenarioReport {
                            name: scenario.name().into(),
                            ..ScenarioReport::default()
                        },
                        Vec::new(),
                        None,
                    )
                } else {
                    let refused = variant
                        .problem(&entry.scenario)
                        .map(|problem| problem.to_string());
                    self.play(&scenario, refused, capture)?
                };
                let recording = ScenarioRecording {
                    name: scenario.name().into(),
                    group: entry.group.clone(),
                    slug,
                    report,
                    skipped,
                    tags: scenario.tag_names().to_vec(),
                    disabled: disabled.clone(),
                    should_fail: scenario.should_fail_reason().map(str::to_owned),
                    pictures: pictures.len(),
                    trace,
                };
                if !skipped && disabled.is_none() {
                    write(&recording, &pictures)?;
                }
                scenarios.push(recording);
            }
        }
        Ok(scenarios)
    }

    /// Plays one scenario in a fresh app and returns its report, pictures and
    /// trace. `refused` is why the scenario may not run in its variant: the
    /// report is then a failed setup, as for any setup that fails. Without
    /// `capture` no pictures are requested and no trace is kept.
    fn play(
        &self,
        scenario: &Scenario,
        refused: Option<String>,
        capture: bool,
    ) -> Result<PlayedScenario, RecordError> {
        let name = scenario.name().to_owned();
        let (width, height) = scenario.window().unwrap_or(DEFAULT_WINDOW_SIZE);
        let mut plugins = RecorderPlugins::default().with_window_size(width, height);
        if let Some(scale_factor) = scenario.scale() {
            plugins = plugins.with_scale_factor(scale_factor);
        }
        let mut app = App::new();
        app.add_plugins(plugins);
        (self.build)(&mut app);
        app.finish();
        app.cleanup();

        // A fresh app, so there is nothing to put back afterwards.
        scenario.fit_rem_size(app.world_mut());
        let setup = refused.map_or_else(|| scenario.run_setup(app.world_mut()), Err);
        if setup.is_ok()
            && let Err(error) = scenario.run_view(app.world_mut())
        {
            bevy::log::warn!("scenario `{name}`: the view could not be arranged: {error}");
        }
        self.settle_app(&mut app, &name)?;

        if capture {
            if let Some(mut captures) = app.world_mut().get_resource_mut::<Captures>() {
                captures.every = self.every;
                captures.enabled = true;
            }
            request_capture(
                app.world_mut(),
                CaptureMeta {
                    frame: 0,
                    steps: Vec::new(),
                    step_text: String::new(),
                    checks_text: String::new(),
                    caption: None,
                    failed: false,
                },
            );
            app.update();
        }
        if let Some(mut player) = app.world_mut().get_resource_mut::<ScenarioPlayer>() {
            if capture {
                player.enable_trace(self.noise.clone());
            }
            player.start_after_setup(scenario.clone(), setup);
        }
        let mut frames = 0;
        while app
            .world()
            .get_resource::<ScenarioPlayer>()
            .is_some_and(ScenarioPlayer::is_active)
        {
            frames += 1;
            if frames > MAX_SCENARIO_FRAMES {
                return Err(RecordError::NeverFinished { scenario: name });
            }
            app.update();
        }
        Self::drain(&mut app, &name)?;
        let report = app
            .world()
            .get_resource::<ScenarioReport>()
            .cloned()
            .unwrap_or_default();
        let pictures = app
            .world_mut()
            .get_resource_mut::<Captures>()
            .map_or_default(|mut captures| core::mem::take(&mut captures.done))
            .into_values()
            .collect();
        let trace = app
            .world_mut()
            .get_resource_mut::<ScenarioPlayer>()
            .and_then(|mut player| player.take_trace());
        Ok((report, pictures, trace))
    }

    /// Runs updates until the app's settle hook is satisfied.
    fn settle_app(&self, app: &mut App, name: &str) -> Result<(), RecordError> {
        for frame in 1..=MAX_SETTLE_FRAMES {
            app.update();
            let settled = self
                .settle
                .as_ref()
                .is_none_or(|settled| settled(app.world_mut()));
            if settled && frame >= MIN_SETTLE_FRAMES {
                return Ok(());
            }
        }
        Err(RecordError::NeverSettled {
            scenario: name.into(),
        })
    }

    /// Runs updates until every picture asked for has come back.
    fn drain(app: &mut App, name: &str) -> Result<(), RecordError> {
        let outstanding = |app: &App| {
            app.world()
                .get_resource::<Captures>()
                .map_or(0, Captures::outstanding)
        };
        for _ in 0..MAX_DRAIN_FRAMES {
            if outstanding(app) == 0 {
                return Ok(());
            }
            app.update();
        }
        match outstanding(app) {
            0 => Ok(()),
            missing => Err(RecordError::NoPictures {
                scenario: name.into(),
                missing,
            }),
        }
    }
}

/// A directory-safe name for a scenario, different from every one in `used`.
fn unique_slug(name: &str, used: &mut Vec<String>) -> String {
    let mut slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    slug = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        slug = "scenario".into();
    }
    let base = slug.clone();
    let mut number = 2;
    while used.contains(&slug) {
        slug = format!("{base}-{number}");
        number += 1;
    }
    used.push(slug.clone());
    slug
}

/// Creates `path` and its parents.
pub(crate) fn create_dir(path: &Path) -> Result<(), RecordError> {
    fs::create_dir_all(path).map_err(|error| RecordError::Io(path.to_path_buf(), error))
}

/// Writes `bytes` to `path`.
pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), RecordError> {
    fs::write(path, bytes).map_err(|error| RecordError::Io(path.to_path_buf(), error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_directory_safe_and_unique() {
        let mut used = Vec::new();
        assert_eq!(
            unique_slug("Click the Pause button!", &mut used),
            "click-the-pause-button"
        );
        assert_eq!(
            unique_slug("Click the pause button", &mut used),
            "click-the-pause-button-2"
        );
        assert_eq!(
            unique_slug("???", &mut used),
            "scenario",
            "nothing usable left"
        );
    }
}
