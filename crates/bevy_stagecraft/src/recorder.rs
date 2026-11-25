//! Recording scenarios offscreen (feature `record`).
//!
//! Without a GPU, set `WGPU_BACKEND=vulkan` and install lavapipe.

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

pub const DEFAULT_EVERY: u32 = 10;
/// Updates run before the scenario starts so layout, the glyph atlas and pipelines are in place.
pub const MIN_SETTLE_FRAMES: u32 = 4;
pub const MAX_SETTLE_FRAMES: u32 = 1200;
pub const MAX_DRAIN_FRAMES: u32 = 240;
/// A script that never ends is an error, not a hang.
pub const MAX_SCENARIO_FRAMES: u32 = 1_000_000;
pub const DEFAULT_GROUP: &str = "Scenarios";

#[derive(Resource, Debug, Clone)]
pub struct RecorderTarget(pub Handle<Image>);

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct CaptureMeta {
    /// `0` is before the first step.
    pub frame: u32,
    pub steps: Vec<usize>,
    pub step_text: String,
    pub checks_text: String,
    pub caption: Option<String>,
    pub failed: bool,
}

#[derive(Debug, Clone)]
pub struct Capture {
    pub meta: CaptureMeta,
    pub frame: Frame,
}

#[derive(Resource, Debug, Default)]
pub struct Captures {
    /// `0` means only at step boundaries and the end.
    pub every: u32,
    pub enabled: bool,
    requested: u32,
    last_requested: Option<u32>,
    done: BTreeMap<u32, Capture>,
}

impl Captures {
    #[must_use]
    pub fn outstanding(&self) -> u32 {
        self.requested
            .saturating_sub(u32::try_from(self.done.len()).unwrap_or(u32::MAX))
    }

    pub fn captures(&self) -> impl Iterator<Item = &Capture> {
        self.done.values()
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecorderSystems {
    Retarget,
    Capture,
}

/// Do not add [`ScenarioPlayerPlugin`] or [`CursorOverlayPlugin`] again; cameras render to an image
/// and the primary window stays virtual.
#[derive(Debug, Clone, Copy)]
pub struct RecorderPlugins {
    window_size: (u32, u32),
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
    #[must_use]
    pub const fn with_window_size(mut self, width: u32, height: u32) -> Self {
        self.window_size = (width, height);
        self
    }

    /// A value that is not finite and above zero is ignored.
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
                // Without this, frame 0 differs from run to run.
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

#[derive(Debug, Default, Clone, Copy)]
struct RecorderSupport;

impl Plugin for RecorderSupport {
    fn build(&self, app: &mut App) {
        app.init_resource::<Captures>()
            .init_resource::<Pictures>()
            .init_resource::<ScreenshotSettings>()
            // The synthetic pointer writes `PointerInput` itself; Bevy's mouse and touch picking
            // would add a second one aimed at the window.
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
                // Name every set that reads a camera's target: Bevy chains them only weakly.
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

#[derive(Component, Debug, Clone, Copy)]
struct CheckPicture(u32);

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
    // Bevy keeps one screenshot per target per frame; reuse the recording's picture.
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

fn show_overlay_again(mut pictures: ResMut<Pictures>, mut roots: Query<&mut Visibility>) {
    for (entity, before) in pictures.take_hidden() {
        if let Ok(mut visibility) = roots.get_mut(entity) {
            *visibility = before;
        }
    }
}

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
    // Keep the data on the CPU side so the handle is also a normal image.
    image.asset_usage = RenderAssetUsages::default();
    let handle = images.add(image);
    input.set_target(Some(NormalizedRenderTarget::Image(image_target(
        &handle,
        scale_factor,
    ))));
    commands.insert_resource(RecorderTarget(handle));
}

/// The cameras' and the pointer's targets must be equal, scale factor included.
fn image_target(handle: &Handle<Image>, scale_factor: f32) -> ImageRenderTarget {
    ImageRenderTarget {
        handle: handle.clone(),
        scale_factor,
    }
}

/// A camera rendering to an image is never Bevy's default UI camera, so mark the one that
/// would have been; otherwise the UI vanishes from the picture.
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

/// Name the image with the cameras' scale factor: Bevy keys the output by the whole target,
/// and any other one is black.
fn screenshot(handle: &Handle<Image>, scale_factor: f32) -> Screenshot {
    Screenshot(RenderTarget::Image(image_target(handle, scale_factor)))
}

fn has_failed(report: &ScenarioReport) -> bool {
    report
        .steps
        .iter()
        .map(|record| &record.outcome)
        .chain(report.checks.iter().map(|record| &record.outcome))
        .any(|outcome| matches!(outcome, Outcome::Fail(_)))
}

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

#[derive(Debug)]
#[non_exhaustive]
pub enum RecordError {
    Io(PathBuf, io::Error),
    Capture(CaptureError),
    NeverSettled {
        scenario: String,
    },
    NeverFinished {
        scenario: String,
    },
    /// Probably no usable GPU; try `WGPU_BACKEND=vulkan` with lavapipe.
    NoPictures {
        scenario: String,
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

#[derive(Debug, Clone)]
pub struct ScenarioRecording {
    pub name: String,
    pub group: String,
    pub slug: String,
    pub report: ScenarioReport,
    pub skipped: bool,
    pub tags: Vec<String>,
    pub disabled: Option<Disabled>,
    pub should_fail: Option<String>,
    pub pictures: usize,
    pub trace: Option<ScenarioTrace>,
}

impl ScenarioRecording {
    #[must_use]
    pub fn passed(&self) -> bool {
        self.report.passed() != self.should_fail.is_some()
    }

    #[must_use]
    pub fn failed_as_expected(&self) -> bool {
        self.should_fail.is_some() && !self.report.passed()
    }

    #[must_use]
    pub const fn skipped(&self) -> bool {
        self.skipped
    }

    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        self.disabled.is_some()
    }

    #[must_use]
    pub fn failed(&self) -> bool {
        !self.passed() && !self.skipped && self.disabled.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct Recording {
    pub directory: PathBuf,
    pub index: PathBuf,
    pub scenarios: Vec<ScenarioRecording>,
}

impl Recording {
    #[must_use]
    pub fn any_failed(&self) -> bool {
        self.scenarios.iter().any(ScenarioRecording::failed)
    }
}

#[derive(Debug, Clone)]
pub struct Checked {
    pub scenarios: Vec<ScenarioRecording>,
}

impl Checked {
    #[must_use]
    pub fn any_failed(&self) -> bool {
        self.scenarios.iter().any(ScenarioRecording::failed)
    }
}

type PlayedScenario = (ScenarioReport, Vec<Capture>, Option<ScenarioTrace>);
type SettleFn = dyn Fn(&mut World) -> bool + Send + Sync;
type BuildFn = dyn Fn(&mut App) + Send + Sync;

struct Entry {
    group: String,
    scenario: Scenario,
}

pub struct Recorder {
    build: Arc<BuildFn>,
    entries: Vec<Entry>,
    settle: Option<Arc<SettleFn>>,
    every: u32,
    noise: NoiseFilter,
    variants: Vec<Variant>,
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

    #[must_use]
    pub fn scenario(self, scenario: Scenario) -> Self {
        self.grouped(DEFAULT_GROUP, scenario)
    }

    #[must_use]
    pub fn grouped(mut self, group: impl Into<String>, scenario: Scenario) -> Self {
        self.entries.push(Entry {
            group: group.into(),
            scenario,
        });
        self
    }

    /// Name variants differently: two with one name share an index title.
    #[must_use]
    pub fn variants(mut self, variants: impl IntoIterator<Item = Variant>) -> Self {
        self.variants = variants.into_iter().collect();
        self
    }

    #[must_use]
    pub fn with_tag_filter(mut self, tags: TagFilter) -> Self {
        self.tags = tags;
        self
    }

    #[must_use]
    pub const fn every(mut self, frames: u32) -> Self {
        self.every = frames;
        self
    }

    #[must_use]
    pub fn with_noise_filter(mut self, noise: NoiseFilter) -> Self {
        self.noise = noise;
        self
    }

    /// At least [`MIN_SETTLE_FRAMES`] updates run whatever it says.
    #[must_use]
    pub fn settle(mut self, settled: impl Fn(&mut World) -> bool + Send + Sync + 'static) -> Self {
        self.settle = Some(Arc::new(settled));
        self
    }

    /// # Errors
    ///
    /// Fails when a file cannot be written, a picture never comes back, or an app never settles
    /// or finishes.
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

    /// Plays as [`Recorder::record`] does but takes no pictures, traces or files; screenshot checks
    /// still use their baselines.
    ///
    /// # Errors
    ///
    /// Fails when an app never settles or never finishes, or a screenshot check's picture never
    /// comes back.
    pub fn check(&self) -> Result<Checked, RecordError> {
        let scenarios = self.run_all(false, |_, _| Ok(()))?;
        Ok(Checked { scenarios })
    }

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

    /// `refused` is why the scenario may not run in its variant; the report is then a failed setup.
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

pub(crate) fn create_dir(path: &Path) -> Result<(), RecordError> {
    fs::create_dir_all(path).map_err(|error| RecordError::Io(path.to_path_buf(), error))
}

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
