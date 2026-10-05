//! What a person watching a scenario sees: the script's cursor and a caption
//! strip naming the step and the checks.
//!
//! While a session is recorded, a checkpoint (see [`crate::session`]) shows as
//! a short flash over the window and a line in the caption strip.
//!
//! In the gallery's pick mode, the node under the pointer is outlined with a
//! tag naming its best target above it, and the picked node has an outline of
//! its own (see [`PickHighlight`]).
//!
//! Only the gallery and the recorder add [`CursorOverlayPlugin`]; headless
//! tests never do. Everything it draws is screen-space UI with
//! [`Pickable::IGNORE`], so it can never take a picking hit from the UI under
//! it.

use alloc::{collections::VecDeque, sync::Arc};
use core::fmt;

use bevy::{
    app::{App, Plugin, Startup, Update},
    asset::{Assets, RenderAssetUsages},
    camera::visibility::Visibility,
    color::{Alpha, Color},
    ecs::{
        component::Component,
        entity::Entity,
        message::MessageReader,
        query::{With, Without},
        resource::Resource,
        schedule::{IntoScheduleConfigs, SystemSet},
        system::{Commands, Local, Query, Res, ResMut},
    },
    image::Image,
    input::{ButtonInput, keyboard::KeyCode, mouse::MouseButton},
    math::Vec2,
    picking::Pickable,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    text::{FontSize, FontSource, TextColor, TextFont},
    ui::{
        AlignItems, BackgroundColor, BorderColor, BorderRadius, ComputedNode, Display,
        FlexDirection, GlobalZIndex, Node, PositionType, UiGlobalTransform, UiRect, UiTargetCamera,
        px,
        widget::{ImageNode, Text},
    },
    window::{PrimaryWindow, Window},
};

use crate::{
    player::{CheckRecord, Outcome, ScenarioReport},
    scenario::{Modifiers, Step},
    session::CheckpointTaken,
    targets::{NotUnderTest, Target},
};

/// Width of the cursor bitmap, in pixels.
pub const CURSOR_WIDTH: u32 = 12;
/// Height of the cursor bitmap, in pixels.
pub const CURSOR_HEIGHT: u32 = 18;
/// How many frames a press ring takes to expand and fade.
pub const RING_FRAMES: u16 = 18;
/// Diameter of a press ring when it appears, in logical pixels.
pub const RING_START_SIZE: f32 = 8.0;
/// Diameter of a press ring when it is gone, in logical pixels.
pub const RING_END_SIZE: f32 = 44.0;
/// Border width of a press ring, in logical pixels.
pub const RING_BORDER: f32 = 3.0;
/// Diameter of the ring shown while a button is held, in logical pixels.
pub const HELD_SIZE: f32 = 26.0;
/// How many past positions the trail remembers.
pub const TRAIL_LENGTH: u16 = 8;
/// Diameter of a trail dot, in logical pixels.
pub const TRAIL_DOT_SIZE: f32 = 6.0;
/// Height of the caption strip, in logical pixels.
pub const CAPTION_HEIGHT: f32 = 56.0;
/// Font size of the caption strip's text, in logical pixels.
pub const CAPTION_FONT_SIZE: f32 = 18.0;
/// Padding inside the caption strip, in logical pixels.
pub const CAPTION_PADDING: f32 = 8.0;
/// How many frames the flash of a checkpoint takes to fade.
pub const CHECKPOINT_FLASH_FRAMES: u16 = 24;
/// How many frames the caption of a checkpoint stays.
pub const CHECKPOINT_CAPTION_FRAMES: u16 = 150;
/// Stacking order of everything the overlay draws, above any app UI.
pub const OVERLAY_Z_INDEX: i32 = i32::MAX - 16;
/// Width of the outline around a node in pick mode, in logical pixels.
pub const PICK_OUTLINE_WIDTH: f32 = 2.0;
/// Font size of the tag over the outlined node, in logical pixels.
pub const PICK_TAG_FONT_SIZE: f32 = 13.0;
/// Height of the tag over the outlined node, in logical pixels.
pub const PICK_TAG_HEIGHT: f32 = 19.0;
/// Padding inside the tag, in logical pixels.
pub const PICK_TAG_PADDING: f32 = 3.0;

/// Default mark before a check that passed. Plain ASCII: rendering showed the
/// bundled font draws the dingbat check marks as empty boxes.
pub const PASS_MARK: &str = "PASS";
/// Default mark before a check that failed; see [`PASS_MARK`].
pub const FAIL_MARK: &str = "FAIL";
/// Default mark before a check that never ran.
pub const SKIPPED_MARK: &str = "-";

/// Colours and sizes of the overlay. Replace the resource to theme it.
#[derive(Resource, Debug, Clone)]
pub struct OverlayTheme {
    /// Tint of the cursor while no button is down.
    pub cursor: Color,
    /// Tint of the cursor while a button is down.
    pub cursor_held: Color,
    /// Colour of the ring shown while a button is down.
    pub held_ring: Color,
    /// Colour of a press ring when it appears; it fades to nothing.
    pub press_ring: Color,
    /// Colour of the newest trail dot; older ones fade.
    pub trail: Color,
    /// Background of the caption strip.
    pub caption_background: Color,
    /// Colour of the text naming the step.
    pub step_text: Color,
    /// Colour of the scenario's own caption.
    pub caption_text: Color,
    /// Colour of a check mark that passed.
    pub pass_text: Color,
    /// Colour of a check mark that failed.
    pub fail_text: Color,
    /// Colour of the flash over the window when a checkpoint is taken; it
    /// fades to nothing.
    pub checkpoint_flash: Color,
    /// Colour of the caption saying a checkpoint was taken.
    pub checkpoint_text: Color,
    /// Colour of the outline around the node under the pointer in pick mode.
    pub pick_hovered: Color,
    /// Colour of the outline around the picked node.
    pub pick_selected: Color,
    /// Background of the tag over the node under the pointer.
    pub pick_tag_background: Color,
    /// Colour of the tag's text.
    pub pick_tag_text: Color,
    /// Size of the caption text.
    pub font_size: f32,
    /// Font of the caption text. The bundled default font only has ASCII, so
    /// the default marks are words; to get `\u{2714}` and `\u{2718}`, set a font
    /// that has them as well as the marks below.
    pub font: FontSource,
    /// Shown before the name of a check that passed.
    pub pass_mark: String,
    /// Shown before the name of a check that failed.
    pub fail_mark: String,
    /// Shown before the name of a check that never ran.
    pub skipped_mark: String,
}

impl Default for OverlayTheme {
    fn default() -> Self {
        Self {
            cursor: Color::WHITE,
            cursor_held: Color::srgb(1.0, 0.75, 0.2),
            held_ring: Color::srgba(1.0, 0.75, 0.2, 0.9),
            press_ring: Color::srgba(1.0, 0.4, 0.2, 0.9),
            trail: Color::srgba(1.0, 1.0, 1.0, 0.5),
            caption_background: Color::srgba(0.05, 0.05, 0.08, 0.85),
            step_text: Color::WHITE,
            caption_text: Color::srgb(0.7, 0.85, 1.0),
            pass_text: Color::srgb(0.4, 0.9, 0.5),
            fail_text: Color::srgb(1.0, 0.4, 0.4),
            checkpoint_flash: Color::srgba(1.0, 0.85, 0.3, 0.35),
            checkpoint_text: Color::srgb(1.0, 0.85, 0.3),
            pick_hovered: Color::srgb(0.2, 0.65, 1.0),
            pick_selected: Color::srgb(1.0, 0.3, 0.75),
            pick_tag_background: Color::srgba(0.05, 0.12, 0.25, 0.92),
            pick_tag_text: Color::WHITE,
            font_size: CAPTION_FONT_SIZE,
            font: FontSource::default(),
            pass_mark: PASS_MARK.into(),
            fail_mark: FAIL_MARK.into(),
            skipped_mark: SKIPPED_MARK.into(),
        }
    }
}

/// How an app's own targets read in the caption strip.
type DescribeFn = dyn Fn(&Target) -> Option<String> + Send + Sync;

/// Words for targets in the caption strip: the extension point for apps whose
/// own targets would otherwise read as the RON a file writes, such as
/// `Station("Alpha")`.
#[derive(Resource, Default, Clone)]
pub struct TargetDescriptions {
    /// Describers, asked in order; the first to answer wins.
    describers: Vec<Arc<DescribeFn>>,
}

impl fmt::Debug for TargetDescriptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TargetDescriptions")
            .field("describers", &self.describers.len())
            .finish()
    }
}

impl TargetDescriptions {
    /// Adds a describer. It returns `None` for targets it has no words for.
    pub fn add(
        &mut self,
        describer: impl Fn(&Target) -> Option<String> + Send + Sync + 'static,
    ) -> &mut Self {
        self.describers.push(Arc::new(describer));
        self
    }

    /// The target in words: an app's describer first, then the built-in one.
    #[must_use]
    pub fn describe(&self, target: &Target) -> String {
        self.describers
            .iter()
            .find_map(|describer| describer(target))
            .unwrap_or_else(|| describe_target(target))
    }
}

/// The built-in words for a target.
#[must_use]
pub fn describe_target(target: &Target) -> String {
    match target {
        Target::Screen(position) => format!("({}, {})", position.x, position.y),
        Target::Text(text) => text.clone(),
        Target::World(position) => format!("({}, {}, {})", position.x, position.y, position.z),
        Target::App(value) => value.to_string(),
        Target::Component(name) | Target::Name(name) => name.clone(),
        Target::Role {
            label: Some(label), ..
        } => label.clone(),
        Target::Role { role, label: None } => format!("{role:?}"),
        Target::Within(_, inner) => describe_target(inner),
        Target::Nth(index, inner) => format!("{} #{index}", describe_target(inner)),
    }
}

/// How a key reads in the caption strip.
fn key_label(key: KeyCode) -> String {
    match key {
        KeyCode::Equal => "=".into(),
        KeyCode::Minus => "-".into(),
        KeyCode::Space => "Space".into(),
        KeyCode::Comma => ",".into(),
        KeyCode::Period => ".".into(),
        KeyCode::Slash => "/".into(),
        other => {
            let name = format!("{other:?}");
            name.strip_prefix("Key")
                .or_else(|| name.strip_prefix("Digit"))
                .unwrap_or(&name)
                .to_owned()
        }
    }
}

/// The keys of a keyboard step with their modifiers, as `Ctrl+Shift+Z`.
fn chord(key: KeyCode, modifiers: Modifiers) -> String {
    let mut text = String::new();
    for held in modifiers.keys() {
        let name = format!("{held:?}");
        let name = name
            .strip_suffix("Left")
            .or_else(|| name.strip_suffix("Right"))
            .unwrap_or(&name);
        text.push_str(name);
        text.push('+');
    }
    text.push_str(&key_label(key));
    text
}

/// How a mouse button reads in a click.
const fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "Click",
        MouseButton::Right => "Right-click",
        MouseButton::Middle => "Middle-click",
        MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => "Click button",
    }
}

/// The target the pointer was last sent to before step `index`, which is what
/// a click or a press then acts on.
fn last_target(steps: &[Step]) -> Option<&Target> {
    steps.iter().rev().find_map(|step| match step.inner() {
        Step::MoveTo(target) | Step::Glide { target, .. } => Some(target),
        Step::Drag { to, .. } => Some(to),
        _ => None,
    })
}

/// One step in words, such as `Click: Rail`, `Glide to (5, 0)` or `Tap =`.
/// `before` is every step that ran earlier, oldest first, which tells a click
/// where it landed.
#[must_use]
pub fn describe_step(step: &Step, before: &[Step], descriptions: &TargetDescriptions) -> String {
    let with_target = |verb: &str| {
        last_target(before).map_or_else(
            || verb.to_owned(),
            |target| format!("{verb}: {}", descriptions.describe(target)),
        )
    };
    match step {
        Step::MoveTo(target) => format!("Move to {}", descriptions.describe(target)),
        Step::Glide { target, .. } => format!("Glide to {}", descriptions.describe(target)),
        Step::Press(_) => with_target("Press"),
        Step::Release(_) => with_target("Release"),
        Step::Click(button) => with_target(button_name(*button)),
        Step::Drag { from, to, .. } => format!(
            "Drag {} to {}",
            descriptions.describe(from),
            descriptions.describe(to)
        ),
        Step::Wheel { x, y } => format!("Scroll ({x}, {y})"),
        Step::Key { key, modifiers } => format!("Hold {}", chord(*key, *modifiers)),
        Step::KeyUp { key, modifiers } => format!("Release {}", chord(*key, *modifiers)),
        Step::Tap { key, modifiers } => format!("Tap {}", chord(*key, *modifiers)),
        Step::Wait(frames) => format!("Wait {frames} frames"),
        Step::Check(check) => format!("Check: {check}"),
        Step::Caption(text) => text.clone(),
        Step::Timeout(frames, inner) => format!(
            "{} (waits up to {frames} frames)",
            describe_step(inner, before, descriptions)
        ),
        Step::Force(inner) => format!("{} (forced)", describe_step(inner, before, descriptions)),
    }
}

/// The step the player is on, in words, or `None` before the first one.
#[must_use]
pub fn current_step_text(
    report: &ScenarioReport,
    descriptions: &TargetDescriptions,
) -> Option<String> {
    let index = report
        .steps
        .iter()
        .rposition(|record| !matches!(record.outcome, Outcome::Skipped))?;
    let record = report.steps.get(index)?;
    let before: Vec<Step> = report
        .steps
        .iter()
        .take(index)
        .map(|record| record.step.clone())
        .collect();
    Some(describe_step(&record.step, &before, descriptions))
}

/// Each check's mark and name, such as `PASS saved  FAIL count`, in the order they
/// ran.
#[must_use]
pub fn checks_text(checks: &[CheckRecord], theme: &OverlayTheme) -> String {
    checks
        .iter()
        .map(|check| {
            let mark = match check.outcome {
                Outcome::Pass => &theme.pass_mark,
                Outcome::Fail(_) => &theme.fail_mark,
                Outcome::Skipped => &theme.skipped_mark,
            };
            format!("{mark} {}", check.name)
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// System sets of the overlay plugin, all in `Update`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlaySystems {
    /// Moves the cursor, rings and trail, and rewrites the caption strip.
    Update,
}

/// The arrow drawn at the synthetic pointer.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCursor;

/// The ring shown while a button is down.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayHeld;

/// A press ring, expanding and fading.
#[derive(Component, Debug, Clone, Copy)]
pub struct OverlayRing {
    /// Frames since the press.
    pub age: u16,
    /// Where the press happened, which the ring stays centred on.
    pub centre: Vec2,
}

/// One dot of the trail; `0` is the newest.
#[derive(Component, Debug, Clone, Copy)]
pub struct OverlayTrailDot(pub u16);

/// The caption strip.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCaptionStrip;

/// The scenario's own caption inside the strip.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCaptionText;

/// The current step inside the strip.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayStepText;

/// The checks inside the strip.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayChecksText;

/// The flash over the window when a checkpoint is taken.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCheckpointFlash;

/// The line in the strip saying a checkpoint was taken.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCheckpointText;

/// Which nodes the overlay outlines in the gallery's pick mode.
///
/// The one under the pointer is outlined with a tag, and the one picked with
/// an outline of its own. The gallery sets it; anything else may too, such
/// as a test that wants the outline in a recorded picture.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct PickHighlight {
    /// The node under the pointer, outlined in
    /// [`OverlayTheme::pick_hovered`].
    pub hovered: Option<Entity>,
    /// The picked node, outlined in [`OverlayTheme::pick_selected`] a little
    /// further out, so both show on the same node.
    pub selected: Option<Entity>,
    /// The words on the tag over the hovered node, such as its best target;
    /// no tag when empty.
    pub tag: String,
}

/// Which of the two pick outlines a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickOutlineKind {
    /// Around the node under the pointer.
    Hovered,
    /// Around the picked node.
    Selected,
}

/// An outline the pick mode draws.
#[derive(Component, Debug, Clone, Copy)]
pub struct OverlayPickOutline(pub PickOutlineKind);

/// The box of the tag over the hovered node.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayPickTag;

/// The text of the tag over the hovered node.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayPickTagText;

/// The last checkpoint, while it is still on show.
#[derive(Resource, Debug, Default)]
struct CheckpointNotice {
    /// Frames since it was taken, and its caption; `None` once it is gone.
    shown: Option<(u16, String)>,
}

/// The caption for a checkpoint, such as `Checkpoint 2: 3 checks suggested`.
#[must_use]
pub fn checkpoint_text(taken: &CheckpointTaken) -> String {
    let checks = match taken.candidates {
        0 => "nothing changed".to_owned(),
        1 => "1 check suggested".to_owned(),
        many => format!("{many} checks suggested"),
    };
    format!("Checkpoint {}: {checks}", taken.number)
}

/// The camera the overlay draws through, for apps that do not leave it to the
/// default UI camera.
///
/// With `None` the overlay's nodes carry no `UiTargetCamera` and go to
/// whichever camera Bevy picks by default. That is the highest-order camera
/// that renders to the primary window; a camera rendering to an image is
/// never the default, so an app whose cameras target images (the recorder
/// retargets them) either marks one with `IsDefaultUiCamera` or sets this.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayCamera(pub Option<Entity>);

/// Every top-level node of the overlay, so one system can aim them all.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayRoot;

/// Where the pointer has been lately, newest first, for the trail.
#[derive(Resource, Debug, Default)]
struct TrailHistory(VecDeque<Vec2>);

/// Draws the script's pointer and a caption strip over the app's UI.
///
/// It draws through the default UI camera unless [`OverlayCamera`] names one.
/// The cursor follows the primary window's cursor position, which
/// `SyntheticInput` sets. The strip reads [`ScenarioReport`]. Replace
/// [`OverlayTheme`] to restyle it and add to [`TargetDescriptions`] to word an
/// app's own targets.
#[derive(Debug, Default, Clone, Copy)]
pub struct CursorOverlayPlugin;

impl Plugin for CursorOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OverlayTheme>()
            .init_resource::<OverlayCamera>()
            .init_resource::<TargetDescriptions>()
            .init_resource::<TrailHistory>()
            .init_resource::<ScenarioReport>()
            .init_resource::<CheckpointNotice>()
            .init_resource::<PickHighlight>()
            .add_message::<CheckpointTaken>()
            .add_systems(
                Startup,
                (spawn_overlay, spawn_checkpoint_flash, spawn_pick_outlines),
            )
            .add_systems(
                Update,
                (
                    aim_overlay,
                    move_cursor,
                    spawn_rings,
                    animate_rings,
                    update_trail,
                    show_checkpoint,
                    show_pick_outlines,
                    update_caption,
                )
                    .chain()
                    .in_set(OverlaySystems::Update),
            );
    }
}

/// Whether the bitmap pixel at `(x, y)` is inside the arrow.
const fn arrow_contains(x: u32, y: u32) -> bool {
    // A triangle down the left edge, leaning on a slope of 3 over 5.
    y < CURSOR_HEIGHT && x * 5 <= y * 3
}

/// The arrow as an RGBA bitmap: a white body with a dark outline, so the tint
/// colours the body and the outline stays readable on any background.
fn arrow_image() -> Image {
    let mut data = Vec::new();
    for y in 0..CURSOR_HEIGHT {
        for x in 0..CURSOR_WIDTH {
            let pixel: [u8; 4] = if !arrow_contains(x, y) {
                [0, 0, 0, 0]
            } else if x == 0 || y + 1 == CURSOR_HEIGHT || !arrow_contains(x + 1, y) {
                [20, 20, 20, 255]
            } else {
                [255, 255, 255, 255]
            };
            data.extend_from_slice(&pixel);
        }
    }
    Image::new(
        Extent3d {
            width: CURSOR_WIDTH,
            height: CURSOR_HEIGHT,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// A round marker node, `size` across, centred on the origin until placed.
fn round_marker(size: f32, border: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: px(size),
        height: px(size),
        border: UiRect::all(px(border)),
        border_radius: BorderRadius::MAX,
        ..Node::default()
    }
}

/// Spawns the cursor, the held ring, the trail and the caption strip.
fn spawn_overlay(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    theme: Res<OverlayTheme>,
) {
    let z = GlobalZIndex(OVERLAY_Z_INDEX);
    for index in 0..TRAIL_LENGTH {
        commands.spawn((
            OverlayTrailDot(index),
            OverlayRoot,
            NotUnderTest,
            round_marker(TRAIL_DOT_SIZE, 0.0),
            BackgroundColor(theme.trail.with_alpha(0.0)),
            Pickable::IGNORE,
            z,
        ));
    }
    commands.spawn((
        OverlayHeld,
        OverlayRoot,
        NotUnderTest,
        round_marker(HELD_SIZE, RING_BORDER),
        BorderColor::all(theme.held_ring),
        Pickable::IGNORE,
        Visibility::Hidden,
        z,
    ));
    commands.spawn((
        OverlayCursor,
        OverlayRoot,
        NotUnderTest,
        Node {
            position_type: PositionType::Absolute,
            width: px(CURSOR_WIDTH),
            height: px(CURSOR_HEIGHT),
            ..Node::default()
        },
        ImageNode {
            color: theme.cursor,
            image: images.add(arrow_image()),
            ..ImageNode::default()
        },
        Pickable::IGNORE,
        Visibility::Hidden,
        GlobalZIndex(OVERLAY_Z_INDEX + 1),
    ));
    let text_font = TextFont {
        font: theme.font.clone(),
        font_size: FontSize::Px(theme.font_size),
        ..TextFont::default()
    };
    commands
        .spawn((
            OverlayCaptionStrip,
            OverlayRoot,
            NotUnderTest,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                bottom: px(0),
                min_height: px(CAPTION_HEIGHT),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Start,
                padding: UiRect::all(px(CAPTION_PADDING)),
                ..Node::default()
            },
            BackgroundColor(theme.caption_background),
            Pickable::IGNORE,
            z,
        ))
        .with_children(|strip| {
            let mut line = |marker: LineMarker, color: Color| {
                let mut line = strip.spawn((
                    Text::new(""),
                    text_font.clone(),
                    TextColor(color),
                    Pickable::IGNORE,
                ));
                match marker {
                    LineMarker::Caption => line.insert(OverlayCaptionText),
                    LineMarker::Step => line.insert(OverlayStepText),
                    LineMarker::Checks => line.insert(OverlayChecksText),
                    LineMarker::Checkpoint => line.insert(OverlayCheckpointText),
                };
            };
            line(LineMarker::Caption, theme.caption_text);
            line(LineMarker::Step, theme.step_text);
            line(LineMarker::Checks, theme.pass_text);
            line(LineMarker::Checkpoint, theme.checkpoint_text);
        });
}

/// Spawns the flash a checkpoint shows, hidden until one is taken.
fn spawn_checkpoint_flash(mut commands: Commands, theme: Res<OverlayTheme>) {
    commands.spawn((
        OverlayCheckpointFlash,
        OverlayRoot,
        NotUnderTest,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            ..Node::default()
        },
        BackgroundColor(theme.checkpoint_flash.with_alpha(0.0)),
        Pickable::IGNORE,
        Visibility::Hidden,
        GlobalZIndex(OVERLAY_Z_INDEX),
    ));
}

/// Spawns the two pick outlines, hidden until [`PickHighlight`] names a
/// node, and the tag over the hovered one.
fn spawn_pick_outlines(mut commands: Commands, theme: Res<OverlayTheme>) {
    for kind in [PickOutlineKind::Selected, PickOutlineKind::Hovered] {
        let colour = match kind {
            PickOutlineKind::Hovered => theme.pick_hovered,
            PickOutlineKind::Selected => theme.pick_selected,
        };
        let mut outline = commands.spawn((
            OverlayPickOutline(kind),
            OverlayRoot,
            NotUnderTest,
            Node {
                position_type: PositionType::Absolute,
                border: UiRect::all(px(PICK_OUTLINE_WIDTH)),
                ..Node::default()
            },
            BorderColor::all(colour),
            Pickable::IGNORE,
            Visibility::Hidden,
            GlobalZIndex(OVERLAY_Z_INDEX),
        ));
        if kind == PickOutlineKind::Hovered {
            outline.with_children(|outline| {
                outline
                    .spawn((
                        OverlayPickTag,
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(-PICK_OUTLINE_WIDTH),
                            height: px(PICK_TAG_HEIGHT),
                            padding: UiRect::axes(px(PICK_TAG_PADDING * 2.0), px(PICK_TAG_PADDING)),
                            align_items: AlignItems::Center,
                            ..Node::default()
                        },
                        BackgroundColor(theme.pick_tag_background),
                        Pickable::IGNORE,
                    ))
                    .with_children(|tag| {
                        tag.spawn((
                            OverlayPickTagText,
                            Text::new(""),
                            TextFont {
                                font: theme.font.clone(),
                                font_size: FontSize::Px(PICK_TAG_FONT_SIZE),
                                ..TextFont::default()
                            },
                            TextColor(theme.pick_tag_text),
                            Pickable::IGNORE,
                        ));
                    });
            });
        }
    }
}

/// The rectangle `entity` takes up, in logical pixels: its top-left corner
/// and its size.
fn logical_rect(
    nodes: &Query<(&ComputedNode, &UiGlobalTransform), Without<OverlayRoot>>,
    entity: Entity,
) -> Option<(Vec2, Vec2)> {
    let (node, transform) = nodes.get(entity).ok()?;
    let size = node.size() * node.inverse_scale_factor;
    let centre = transform.affine().translation * node.inverse_scale_factor;
    Some((centre - size / 2.0, size))
}

/// Puts the pick outlines around the nodes [`PickHighlight`] names, or hides
/// them, and writes the tag.
fn show_pick_outlines(
    highlight: Res<PickHighlight>,
    theme: Res<OverlayTheme>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform), Without<OverlayRoot>>,
    mut outlines: Query<(
        &OverlayPickOutline,
        &mut Node,
        &mut Visibility,
        &mut BorderColor,
    )>,
    mut tags: Query<
        (&mut Node, &mut Visibility, &mut BackgroundColor),
        (With<OverlayPickTag>, Without<OverlayPickOutline>),
    >,
    mut texts: Query<(&mut Text, &mut TextColor), With<OverlayPickTagText>>,
) {
    let mut hovered_top = None;
    for (OverlayPickOutline(kind), mut node, mut visibility, mut border) in &mut outlines {
        let (entity, colour, gap) = match kind {
            PickOutlineKind::Hovered => (highlight.hovered, theme.pick_hovered, 0.0),
            PickOutlineKind::Selected => {
                (highlight.selected, theme.pick_selected, PICK_OUTLINE_WIDTH)
            }
        };
        let Some((corner, size)) = entity.and_then(|entity| logical_rect(&nodes, entity)) else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        // Drawn just outside the node, so the node itself stays readable.
        let out = PICK_OUTLINE_WIDTH + gap;
        node.left = px(corner.x - out);
        node.top = px(corner.y - out);
        node.width = px(2.0f32.mul_add(out, size.x));
        node.height = px(2.0f32.mul_add(out, size.y));
        *border = BorderColor::all(colour);
        *visibility = Visibility::Inherited;
        if *kind == PickOutlineKind::Hovered {
            hovered_top = Some((corner.y - out, 2.0f32.mul_add(out, size.y)));
        }
    }
    let shown = hovered_top.filter(|_| !highlight.tag.is_empty());
    for (mut node, mut visibility, mut background) in &mut tags {
        let Some((top, height)) = shown else {
            *visibility = Visibility::Hidden;
            continue;
        };
        // Above the outline, or below it where there is no room above. An
        // absolute child is placed from inside its parent's border.
        node.top = if top >= PICK_TAG_HEIGHT {
            px(-(PICK_TAG_HEIGHT + PICK_OUTLINE_WIDTH))
        } else {
            px(height - PICK_OUTLINE_WIDTH)
        };
        background.0 = theme.pick_tag_background;
        *visibility = Visibility::Inherited;
    }
    for (mut text, mut colour) in &mut texts {
        if text.0 != highlight.tag {
            text.0.clone_from(&highlight.tag);
        }
        colour.0 = theme.pick_tag_text;
    }
}

/// Points every overlay root at [`OverlayCamera`], when one is named.
fn aim_overlay(
    mut commands: Commands,
    camera: Res<OverlayCamera>,
    roots: Query<(Entity, Option<&UiTargetCamera>), With<OverlayRoot>>,
) {
    let Some(camera) = camera.0 else {
        return;
    };
    for (entity, aimed) in &roots {
        if aimed.is_none_or(|aimed| aimed.0 != camera) {
            commands.entity(entity).insert(UiTargetCamera(camera));
        }
    }
}

/// Which line of the caption strip is being spawned.
#[derive(Debug, Clone, Copy)]
enum LineMarker {
    /// The scenario's own caption.
    Caption,
    /// The current step.
    Step,
    /// The checks.
    Checks,
    /// The last checkpoint.
    Checkpoint,
}

/// The pointer's position, or `None` before the script has moved it.
fn pointer_position(windows: &Query<&Window, With<PrimaryWindow>>) -> Option<Vec2> {
    windows.iter().next().and_then(Window::cursor_position)
}

/// Puts the arrow and the held ring at the pointer and tints them.
fn move_cursor(
    windows: Query<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    theme: Res<OverlayTheme>,
    mut cursor: Query<
        (&mut Node, &mut ImageNode, &mut Visibility),
        (With<OverlayCursor>, Without<OverlayHeld>),
    >,
    mut held: Query<
        (&mut Node, &mut Visibility, &mut BorderColor),
        (With<OverlayHeld>, Without<OverlayCursor>),
    >,
) {
    let position = pointer_position(&windows);
    let down = buttons.pressed(MouseButton::Left)
        || buttons.pressed(MouseButton::Right)
        || buttons.pressed(MouseButton::Middle);
    let visibility = |shown: bool| {
        if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    };
    for (mut node, mut image, mut shown) in &mut cursor {
        *shown = visibility(position.is_some());
        if let Some(position) = position {
            node.left = px(position.x);
            node.top = px(position.y);
        }
        image.color = if down {
            theme.cursor_held
        } else {
            theme.cursor
        };
    }
    for (mut node, mut shown, mut border) in &mut held {
        *shown = visibility(position.is_some() && down);
        *border = BorderColor::all(theme.held_ring);
        if let Some(position) = position {
            node.left = px(position.x - HELD_SIZE / 2.0);
            node.top = px(position.y - HELD_SIZE / 2.0);
        }
    }
}

/// Starts a ring wherever a button went down this frame.
fn spawn_rings(
    mut commands: Commands,
    windows: Query<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    theme: Res<OverlayTheme>,
    camera: Res<OverlayCamera>,
) {
    let pressed = buttons.just_pressed(MouseButton::Left)
        || buttons.just_pressed(MouseButton::Right)
        || buttons.just_pressed(MouseButton::Middle);
    let Some(position) = pointer_position(&windows).filter(|_| pressed) else {
        return;
    };
    let mut ring = commands.spawn((
        OverlayRing {
            age: 0,
            centre: position,
        },
        OverlayRoot,
        NotUnderTest,
        Node {
            left: px(position.x - RING_START_SIZE / 2.0),
            top: px(position.y - RING_START_SIZE / 2.0),
            ..round_marker(RING_START_SIZE, RING_BORDER)
        },
        BorderColor::all(theme.press_ring),
        Pickable::IGNORE,
        GlobalZIndex(OVERLAY_Z_INDEX),
    ));
    if let Some(camera) = camera.0 {
        ring.insert(UiTargetCamera(camera));
    }
}

/// Grows and fades the rings, and removes the ones that are done. Counts
/// frames, not seconds, so every runner draws the same thing.
fn animate_rings(
    mut commands: Commands,
    theme: Res<OverlayTheme>,
    mut rings: Query<(Entity, &mut OverlayRing, &mut Node, &mut BorderColor)>,
) {
    for (entity, mut ring, mut node, mut border) in &mut rings {
        ring.age += 1;
        if ring.age >= RING_FRAMES {
            commands.entity(entity).despawn();
            continue;
        }
        let progress = f32::from(ring.age) / f32::from(RING_FRAMES);
        let size = (RING_END_SIZE - RING_START_SIZE).mul_add(progress, RING_START_SIZE);
        let centre = ring.centre;
        node.width = px(size);
        node.height = px(size);
        node.left = px(centre.x - size / 2.0);
        node.top = px(centre.y - size / 2.0);
        *border = BorderColor::all(
            theme
                .press_ring
                .with_alpha(theme.press_ring.alpha() * (1.0 - progress)),
        );
    }
}

/// Remembers where the pointer has been and fades the dots along that path.
/// When the pointer rests, the path shortens until the trail is gone.
fn update_trail(
    windows: Query<&Window, With<PrimaryWindow>>,
    theme: Res<OverlayTheme>,
    mut history: ResMut<TrailHistory>,
    mut dots: Query<(&OverlayTrailDot, &mut Node, &mut BackgroundColor)>,
    mut last: Local<Option<Vec2>>,
) {
    let position = pointer_position(&windows);
    match position {
        Some(position) if *last != Some(position) => {
            history.0.push_front(position);
            history.0.truncate(usize::from(TRAIL_LENGTH) + 1);
        }
        _ => {
            history.0.pop_back();
        }
    }
    *last = position;
    for (dot, mut node, mut colour) in &mut dots {
        // Index 0 is where the arrow is now, so the dots start behind it.
        let Some(at) = history.0.get(usize::from(dot.0) + 1) else {
            *colour = BackgroundColor(theme.trail.with_alpha(0.0));
            continue;
        };
        let fade = 1.0 - (f32::from(dot.0) / f32::from(TRAIL_LENGTH));
        node.left = px(at.x - TRAIL_DOT_SIZE / 2.0);
        node.top = px(at.y - TRAIL_DOT_SIZE / 2.0);
        *colour = BackgroundColor(theme.trail.with_alpha(theme.trail.alpha() * fade));
    }
}

/// Flashes the window and names the checkpoint when one is taken, and fades
/// both out. Counts frames, not seconds, so every runner draws the same thing.
fn show_checkpoint(
    mut taken: MessageReader<CheckpointTaken>,
    mut notice: ResMut<CheckpointNotice>,
    theme: Res<OverlayTheme>,
    mut flash: Query<(&mut BackgroundColor, &mut Visibility), With<OverlayCheckpointFlash>>,
    mut line: Query<(&mut Text, &mut TextColor), With<OverlayCheckpointText>>,
) {
    if let Some(latest) = taken.read().last() {
        notice.shown = Some((0, checkpoint_text(latest)));
    } else if let Some((age, _)) = notice.shown.as_mut() {
        *age = age.saturating_add(1);
    }
    if notice
        .shown
        .as_ref()
        .is_some_and(|(age, _)| *age >= CHECKPOINT_CAPTION_FRAMES)
    {
        notice.shown = None;
    }
    let age = notice.shown.as_ref().map(|(age, _)| *age);
    let fading = age.filter(|age| *age < CHECKPOINT_FLASH_FRAMES);
    for (mut colour, mut visibility) in &mut flash {
        if let Some(age) = fading {
            let left = 1.0 - f32::from(age) / f32::from(CHECKPOINT_FLASH_FRAMES);
            colour.0 = theme
                .checkpoint_flash
                .with_alpha(theme.checkpoint_flash.alpha() * left);
            *visibility = Visibility::Inherited;
        } else if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
    }
    let caption = notice
        .shown
        .as_ref()
        .map_or("", |(_, caption)| caption.as_str());
    for (mut text, mut colour) in &mut line {
        if text.0 != caption {
            caption.clone_into(&mut text.0);
        }
        colour.0 = theme.checkpoint_text;
    }
}

/// Rewrites the strip from the report.
fn update_caption(
    report: Res<ScenarioReport>,
    notice: Res<CheckpointNotice>,
    theme: Res<OverlayTheme>,
    descriptions: Res<TargetDescriptions>,
    mut strip: Query<&mut Node, With<OverlayCaptionStrip>>,
    mut caption: Query<
        &mut Text,
        (
            With<OverlayCaptionText>,
            Without<OverlayStepText>,
            Without<OverlayChecksText>,
        ),
    >,
    mut step: Query<
        &mut Text,
        (
            With<OverlayStepText>,
            Without<OverlayCaptionText>,
            Without<OverlayChecksText>,
        ),
    >,
    mut checks: Query<
        (&mut Text, &mut TextColor),
        (
            With<OverlayChecksText>,
            Without<OverlayCaptionText>,
            Without<OverlayStepText>,
        ),
    >,
) {
    let step_text = current_step_text(&report, &descriptions).unwrap_or_default();
    let checks_text = checks_text(&report.checks, &theme);
    let failed = report
        .checks
        .iter()
        .any(|check| matches!(check.outcome, Outcome::Fail(_)));
    for mut text in &mut caption {
        text.0
            .clone_from(report.caption.as_ref().unwrap_or(&String::new()));
    }
    for mut text in &mut step {
        text.0.clone_from(&step_text);
    }
    for (mut text, mut colour) in &mut checks {
        text.0.clone_from(&checks_text);
        colour.0 = if failed {
            theme.fail_text
        } else {
            theme.pass_text
        };
    }
    for mut node in &mut strip {
        node.display = if step_text.is_empty() && checks_text.is_empty() && notice.shown.is_none() {
            Display::None
        } else {
            Display::Flex
        };
    }
}
