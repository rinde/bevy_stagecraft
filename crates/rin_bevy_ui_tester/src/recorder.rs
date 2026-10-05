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
    app::{App, Last, Plugin, PluginGroup, PluginGroupBuilder, PostUpdate, Startup},
    asset::{Assets, Handle, RenderAssetUsages},
    camera::{Camera, CameraUpdateSystems, NormalizedRenderTarget, RenderTarget},
    core_pipeline::CorePipelinePlugin,
    ecs::{
        component::Component,
        entity::Entity,
        observer::On,
        query::With,
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
    capture::{CaptureError, Frame},
    check::suggest::NoiseFilter,
    headless::{DEFAULT_WINDOW_SIZE, HeadlessPlugins, RenderCameraSupport},
    index,
    input::SyntheticInput,
    overlay::{CursorOverlayPlugin, TargetDescriptions, current_step_text},
    player::{Outcome, ScenarioPlayer, ScenarioPlayerPlugin, ScenarioReport, ScenarioSystems},
    scenario::Scenario,
    trace::ScenarioTrace,
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
    /// Size of the virtual primary window and of the image.
    window_size: (u32, u32),
}

impl Default for RecorderPlugins {
    fn default() -> Self {
        Self {
            window_size: DEFAULT_WINDOW_SIZE,
        }
    }
}

impl RecorderPlugins {
    /// Sets the size of the window and the image, in logical pixels.
    #[must_use]
    pub const fn with_window_size(mut self, width: u32, height: u32) -> Self {
        self.window_size = (width, height);
        self
    }
}

impl PluginGroup for RecorderPlugins {
    fn build(self) -> PluginGroupBuilder {
        let (width, height) = self.window_size;
        HeadlessPlugins::default()
            .with_window_size(width, height)
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
            // The synthetic pointer writes `PointerInput` itself; Bevy's own
            // mouse picking would add a second one aimed at the window, which
            // nothing renders to, and the last one wins.
            .insert_resource(PointerInputSettings {
                is_mouse_enabled: false,
                is_touch_enabled: false,
            })
            .add_observer(store_capture)
            .add_systems(Startup, create_target)
            .add_systems(
                PostUpdate,
                retarget_cameras
                    .in_set(RecorderSystems::Retarget)
                    .before(CameraUpdateSystems)
                    .before(UiSystems::Prepare),
            )
            .add_systems(
                Last,
                // After the trace point, so a point's frame gets its picture.
                request_frames
                    .in_set(RecorderSystems::Capture)
                    .after(ScenarioSystems::Trace),
            );
    }
}

/// Creates the image and aims the synthetic pointer at it.
fn create_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut input: ResMut<SyntheticInput>,
) {
    let (width, height) = windows.iter().next().map_or(DEFAULT_WINDOW_SIZE, |window| {
        (
            window.resolution.physical_width(),
            window.resolution.physical_height(),
        )
    });
    let mut image = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    // Read back through the screenshot's own copy, but keep the data on the
    // CPU side so the handle is also a normal image.
    image.asset_usage = RenderAssetUsages::default();
    let handle = images.add(image);
    input.set_target(Some(NormalizedRenderTarget::Image(handle.clone().into())));
    commands.insert_resource(RecorderTarget(handle));
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
    primary: Query<Entity, With<PrimaryWindow>>,
) {
    let Some(target) = target else {
        return;
    };
    let default = cameras.p0().get();
    let primary = primary.iter().next();
    for (entity, mut render_target) in &mut cameras.p1() {
        let to_window = match &*render_target {
            RenderTarget::Window(WindowRef::Primary) => true,
            RenderTarget::Window(WindowRef::Entity(window)) => Some(*window) == primary,
            _ => false,
        };
        if !to_window {
            continue;
        }
        *render_target = RenderTarget::Image(target.0.clone().into());
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
    commands.spawn((Screenshot::image(target.0.clone()), meta));
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
    world.spawn((Screenshot::image(target.0), meta));
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
        self.report.passed()
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
    /// Whether any scenario failed, for the calling binary to exit non-zero.
    #[must_use]
    pub fn any_failed(&self) -> bool {
        self.scenarios.iter().any(|scenario| !scenario.passed())
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
        let mut used = Vec::new();
        let mut scenarios = Vec::new();
        for entry in &self.entries {
            let slug = unique_slug(entry.scenario.name(), &mut used);
            let (report, pictures, trace) = self.play(&entry.scenario)?;
            let folder = directory.join(&slug);
            let recording = ScenarioRecording {
                name: entry.scenario.name().into(),
                group: entry.group.clone(),
                slug,
                report,
                pictures: pictures.len(),
                trace,
            };
            index::write_scenario(&folder, &pictures, &recording)?;
            scenarios.push(recording);
        }
        let index = directory.join("index.html");
        write_file(&index, index::index_html(&scenarios).as_bytes())?;
        Ok(Recording {
            directory: directory.to_path_buf(),
            index,
            scenarios,
        })
    }

    /// Plays one scenario in a fresh app and returns its report, pictures and
    /// trace.
    fn play(&self, scenario: &Scenario) -> Result<PlayedScenario, RecordError> {
        let name = scenario.name().to_owned();
        let (width, height) = scenario.window().unwrap_or(DEFAULT_WINDOW_SIZE);
        let mut app = App::new();
        app.add_plugins(RecorderPlugins::default().with_window_size(width, height));
        (self.build)(&mut app);
        app.finish();
        app.cleanup();

        let setup = scenario.run_setup(app.world_mut());
        if setup.is_ok()
            && let Err(error) = scenario.run_view(app.world_mut())
        {
            bevy::log::warn!("scenario `{name}`: the view could not be arranged: {error}");
        }
        self.settle_app(&mut app, &name)?;

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
        if let Some(mut player) = app.world_mut().get_resource_mut::<ScenarioPlayer>() {
            player.enable_trace(self.noise.clone());
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
            .map(|mut captures| core::mem::take(&mut captures.done))
            .unwrap_or_default()
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
