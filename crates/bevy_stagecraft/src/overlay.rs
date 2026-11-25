//! Screen-space UI with [`Pickable::IGNORE`], so it never takes a picking hit.

use alloc::{collections::VecDeque, sync::Arc};
use core::fmt;

use bevy::{
    app::{App, Plugin, Update},
    asset::{Assets, RenderAssetUsages},
    camera::{Camera, visibility::Visibility},
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
    input::{ButtonInput, keyboard::KeyCode, mouse::MouseButton, touch::Touches},
    math::Vec2,
    picking::Pickable,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    text::{FontSize, FontSource, TextColor, TextFont},
    ui::{
        AlignItems, BackgroundColor, BorderColor, BorderRadius, ComputedNode, DefaultUiCamera,
        Display, FlexDirection, GlobalZIndex, Node, PositionType, UiGlobalTransform, UiRect,
        UiTargetCamera, px,
        widget::{ImageNode, Text},
    },
    window::{PrimaryWindow, Window},
};

use crate::{
    player::{CheckRecord, Outcome, ScenarioReport},
    scenario::{Modifiers, Step, flatten, with_group_path},
    session::{CheckpointTaken, SessionRecorder},
    targets::{NotUnderTest, Target},
};

pub const CURSOR_WIDTH: u32 = 12;
pub const CURSOR_HEIGHT: u32 = 18;
pub const RING_FRAMES: u16 = 18;
pub const RING_START_SIZE: f32 = 8.0;
pub const RING_END_SIZE: f32 = 44.0;
pub const RING_BORDER: f32 = 3.0;
pub const HELD_SIZE: f32 = 26.0;
pub const TRAIL_LENGTH: u16 = 8;
pub const TRAIL_DOT_SIZE: f32 = 6.0;
pub const FINGER_SIZE: f32 = 30.0;
pub const TOUCH_MARK_FRAMES: u16 = 24;
pub const TOUCH_MARK_START_SIZE: f32 = 56.0;
pub const TOUCH_MARK_END_SIZE: f32 = 36.0;
pub const TOUCH_MARK_BORDER: f32 = 4.0;
pub const CAPTION_HEIGHT: f32 = 56.0;
pub const CAPTION_FONT_SIZE: f32 = 18.0;
pub const CAPTION_PADDING: f32 = 8.0;
pub const CHECKPOINT_FLASH_FRAMES: u16 = 24;
pub const CHECKPOINT_CAPTION_FRAMES: u16 = 150;
pub const OVERLAY_Z_INDEX: i32 = i32::MAX - 16;
pub const PICK_OUTLINE_WIDTH: f32 = 2.0;
pub const PICK_TAG_FONT_SIZE: f32 = 13.0;
pub const PICK_TAG_HEIGHT: f32 = 19.0;
pub const PICK_TAG_PADDING: f32 = 3.0;

/// Plain ASCII: the bundled font draws the dingbat check marks as empty boxes.
pub const PASS_MARK: &str = "PASS";
pub const FAIL_MARK: &str = "FAIL";
pub const SKIPPED_MARK: &str = "-";

#[derive(Resource, Debug, Clone)]
pub struct OverlayTheme {
    pub cursor: Color,
    pub cursor_held: Color,
    pub held_ring: Color,
    pub finger: Color,
    pub touch_down_mark: Color,
    pub touch_up_mark: Color,
    pub press_ring: Color,
    pub trail: Color,
    pub caption_background: Color,
    pub step_text: Color,
    pub caption_text: Color,
    pub pass_text: Color,
    pub fail_text: Color,
    pub checkpoint_flash: Color,
    pub checkpoint_text: Color,
    pub pick_hovered: Color,
    pub pick_selected: Color,
    pub pick_tag_background: Color,
    pub pick_tag_text: Color,
    pub font_size: f32,
    /// The bundled font only has ASCII; to use `\u{2714}` and `\u{2718}`, set a font
    /// that has them along with the marks.
    pub font: FontSource,
    pub pass_mark: String,
    pub fail_mark: String,
    pub skipped_mark: String,
}

impl Default for OverlayTheme {
    fn default() -> Self {
        Self {
            cursor: Color::WHITE,
            cursor_held: Color::srgb(1.0, 0.75, 0.2),
            held_ring: Color::srgba(1.0, 0.75, 0.2, 0.9),
            finger: Color::srgba(0.2, 0.8, 0.9, 0.55),
            touch_down_mark: Color::srgba(0.3, 0.9, 0.4, 0.95),
            touch_up_mark: Color::srgba(0.7, 0.4, 1.0, 0.95),
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

type DescribeFn = dyn Fn(&Target) -> Option<String> + Send + Sync;

#[derive(Resource, Default, Clone)]
pub struct TargetDescriptions {
    /// The first to answer wins.
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
    pub fn add(
        &mut self,
        describer: impl Fn(&Target) -> Option<String> + Send + Sync + 'static,
    ) -> &mut Self {
        self.describers.push(Arc::new(describer));
        self
    }

    #[must_use]
    pub fn describe(&self, target: &Target) -> String {
        self.describers
            .iter()
            .find_map(|describer| describer(target))
            .unwrap_or_else(|| describe_target(target))
    }
}

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

const fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "Click",
        MouseButton::Right => "Right-click",
        MouseButton::Middle => "Middle-click",
        MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => "Click button",
    }
}

fn finger_suffix(finger: u8) -> String {
    if finger == 0 {
        String::new()
    } else {
        format!(" (finger {finger})")
    }
}

fn last_target(steps: &[Step]) -> Option<&Target> {
    steps.iter().rev().find_map(|step| match step.inner() {
        Step::MoveTo(target)
        | Step::Glide { target, .. }
        | Step::Type {
            into: Some(target), ..
        }
        | Step::Drag { to: target, .. } => Some(target),
        _ => None,
    })
}

/// `before` is every step that ran earlier, oldest first; it tells a click where
/// it landed.
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
        Step::TouchDown { at, finger } => format!(
            "Touch down on {}{}",
            descriptions.describe(at),
            finger_suffix(*finger)
        ),
        Step::TouchMove { to, finger } => format!(
            "Touch move to {}{}",
            descriptions.describe(to),
            finger_suffix(*finger)
        ),
        Step::TouchUp { finger } => format!("Touch up{}", finger_suffix(*finger)),
        Step::TouchCancel { finger } => format!("Touch cancel{}", finger_suffix(*finger)),
        Step::TouchTap(target) => format!("Tap {}", descriptions.describe(target)),
        Step::TouchDrag { from, to, .. } => format!(
            "Touch drag {} to {}",
            descriptions.describe(from),
            descriptions.describe(to)
        ),
        Step::Wheel { x, y } => format!("Scroll ({x}, {y})"),
        Step::MoveBurst(positions) => format!("Move burst of {} positions", positions.len()),
        Step::Key { key, modifiers } => format!("Hold {}", chord(*key, *modifiers)),
        Step::KeyUp { key, modifiers } => format!("Release {}", chord(*key, *modifiers)),
        Step::Tap { key, modifiers } => format!("Tap {}", chord(*key, *modifiers)),
        Step::Type { text, into: None } => format!("Type {text:?}"),
        Step::Type {
            text,
            into: Some(target),
        } => format!("Type {text:?} into {}", descriptions.describe(target)),
        Step::Wait(frames) => format!("Wait {frames} frames"),
        Step::Check(check) => format!("Check: {check}"),
        Step::Capture { name, value } => format!("Capture {name:?}: {value}"),
        Step::Caption(text) => text.clone(),
        Step::Timeout(frames, inner) => format!(
            "{} (waits up to {frames} frames)",
            describe_step(inner, before, descriptions)
        ),
        Step::Force(inner) => format!("{} (forced)", describe_step(inner, before, descriptions)),
        Step::Group { name, steps } => format!("{name} ({} steps)", flatten(steps).len()),
    }
}

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
    Some(with_group_path(
        &record.group,
        &describe_step(&record.step, &before, descriptions),
    ))
}

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

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlaySystems {
    Update,
}

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCursor;

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayHeld;

#[derive(Component, Debug, Clone, Copy)]
pub struct OverlayRing {
    pub age: u16,
    pub center: Vec2,
}

/// Exists exactly while the finger is down. Not the arrow: a touch step never
/// moves the mouse cursor.
#[derive(Component, Debug, Clone, Copy)]
pub struct OverlayFinger(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchMarkKind {
    Down,
    Up,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct OverlayTouchMark {
    pub kind: TouchMarkKind,
    pub age: u16,
    pub center: Vec2,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct OverlayTrailDot(pub u16);

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCaptionStrip;

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCaptionText;

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayStepText;

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayChecksText;

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCheckpointFlash;

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayCheckpointText;

#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct PickHighlight {
    pub hovered: Option<Entity>,
    /// Outlined a little further out, so both show on the same node.
    pub selected: Option<Entity>,
    pub tag: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickOutlineKind {
    Hovered,
    Selected,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct OverlayPickOutline(pub PickOutlineKind);

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayPickTag;

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayPickTagText;

#[derive(Resource, Debug, Default)]
struct CheckpointNotice {
    shown: Option<(u16, String)>,
}

#[must_use]
pub fn checkpoint_text(taken: &CheckpointTaken) -> String {
    let checks = match taken.candidates {
        0 => "nothing changed".to_owned(),
        1 => "1 check suggested".to_owned(),
        many => format!("{many} checks suggested"),
    };
    format!("Checkpoint {}: {checks}", taken.number)
}

/// ASCII only: the bundled font has no more.
#[must_use]
pub fn naming_text(number: usize, typed: &str) -> String {
    format!("Name checkpoint {number} (Enter keeps, Esc skips): {typed}_")
}

/// `None` aims at Bevy's default UI camera, which never renders to an image: set
/// this, or mark one `IsDefaultUiCamera`, when the recorder retargets cameras.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayCamera(pub Option<Entity>);

#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
struct OverlayAim(Option<Entity>);

/// Carries `UiTargetCamera` from its spawn bundle, so an app's `Add<Node>`
/// observer can tell it from the app's own roots.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayRoot;

#[derive(Resource, Debug, Default)]
struct TrailHistory(VecDeque<Vec2>);

#[derive(Debug, Default, Clone, Copy)]
pub struct CursorOverlayPlugin;

impl Plugin for CursorOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OverlayTheme>()
            .init_resource::<OverlayCamera>()
            .init_resource::<OverlayAim>()
            .init_resource::<TargetDescriptions>()
            .init_resource::<TrailHistory>()
            .init_resource::<ScenarioReport>()
            .init_resource::<CheckpointNotice>()
            .init_resource::<PickHighlight>()
            .add_message::<CheckpointTaken>()
            .add_systems(
                Update,
                (
                    maintain_overlay,
                    move_cursor,
                    spawn_rings,
                    animate_rings,
                    show_fingers,
                    animate_touch_marks,
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

const fn arrow_contains(x: u32, y: u32) -> bool {
    y < CURSOR_HEIGHT && x * 5 <= y * 3
}

/// A white body with a dark outline, so the tint colors the body and the outline
/// stays readable on any background.
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

fn spawn_overlay(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    theme: &OverlayTheme,
    camera: Entity,
) {
    let target = UiTargetCamera(camera);
    let z = GlobalZIndex(OVERLAY_Z_INDEX);
    for index in 0..TRAIL_LENGTH {
        commands.spawn((
            OverlayTrailDot(index),
            OverlayRoot,
            NotUnderTest,
            target.clone(),
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
        target.clone(),
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
        target.clone(),
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
            target,
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

fn spawn_checkpoint_flash(commands: &mut Commands, theme: &OverlayTheme, camera: Entity) {
    commands.spawn((
        OverlayCheckpointFlash,
        OverlayRoot,
        NotUnderTest,
        UiTargetCamera(camera),
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

fn spawn_pick_outlines(commands: &mut Commands, theme: &OverlayTheme, camera: Entity) {
    for kind in [PickOutlineKind::Selected, PickOutlineKind::Hovered] {
        let color = match kind {
            PickOutlineKind::Hovered => theme.pick_hovered,
            PickOutlineKind::Selected => theme.pick_selected,
        };
        let mut outline = commands.spawn((
            OverlayPickOutline(kind),
            OverlayRoot,
            NotUnderTest,
            UiTargetCamera(camera),
            Node {
                position_type: PositionType::Absolute,
                border: UiRect::all(px(PICK_OUTLINE_WIDTH)),
                ..Node::default()
            },
            BorderColor::all(color),
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

fn logical_rect(
    nodes: &Query<(&ComputedNode, &UiGlobalTransform), Without<OverlayRoot>>,
    entity: Entity,
) -> Option<(Vec2, Vec2)> {
    let (node, transform) = nodes.get(entity).ok()?;
    let size = node.size() * node.inverse_scale_factor;
    let center = transform.affine().translation * node.inverse_scale_factor;
    Some((center - size / 2.0, size))
}

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
        let (entity, color, gap) = match kind {
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
        *border = BorderColor::all(color);
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
    for (mut text, mut color) in &mut texts {
        if text.0 != highlight.tag {
            text.0.clone_from(&highlight.tag);
        }
        color.0 = theme.pick_tag_text;
    }
}

fn wanted_camera(
    named: OverlayCamera,
    default: &DefaultUiCamera,
    cameras: &Query<(), With<Camera>>,
) -> Option<Entity> {
    named.0.map_or_else(
        || default.get(),
        |camera| cameras.contains(camera).then_some(camera),
    )
}

/// Spawn roots only once a camera exists: each must carry `UiTargetCamera` from
/// its spawn bundle, or an app's `Add<Node>` observer takes it for the app's.
fn maintain_overlay(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut aim: ResMut<OverlayAim>,
    named: Res<OverlayCamera>,
    default: DefaultUiCamera,
    cameras: Query<(), With<Camera>>,
    theme: Res<OverlayTheme>,
    roots: Query<(Entity, Option<&UiTargetCamera>), With<OverlayRoot>>,
) {
    let wanted = wanted_camera(*named, &default, &cameras);
    match (aim.0, wanted) {
        (None, Some(camera)) => {
            spawn_overlay(&mut commands, &mut images, &theme, camera);
            spawn_checkpoint_flash(&mut commands, &theme, camera);
            spawn_pick_outlines(&mut commands, &theme, camera);
        }
        (Some(_), None) => {
            for (entity, _) in &roots {
                commands.entity(entity).despawn();
            }
        }
        (Some(_), Some(camera)) => {
            for (entity, aimed) in &roots {
                if aimed.is_none_or(|aimed| aimed.0 != camera) {
                    commands.entity(entity).insert(UiTargetCamera(camera));
                }
            }
        }
        (None, None) => {}
    }
    if aim.0 != wanted {
        aim.0 = wanted;
    }
}

#[derive(Debug, Clone, Copy)]
enum LineMarker {
    Caption,
    Step,
    Checks,
    Checkpoint,
}

fn pointer_position(windows: &Query<&Window, With<PrimaryWindow>>) -> Option<Vec2> {
    windows.iter().next().and_then(Window::cursor_position)
}

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

fn spawn_rings(
    mut commands: Commands,
    windows: Query<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    theme: Res<OverlayTheme>,
    aim: Res<OverlayAim>,
) {
    let pressed = buttons.just_pressed(MouseButton::Left)
        || buttons.just_pressed(MouseButton::Right)
        || buttons.just_pressed(MouseButton::Middle);
    let (Some(position), Some(camera)) = (pointer_position(&windows).filter(|_| pressed), aim.0)
    else {
        return;
    };
    commands.spawn((
        OverlayRing {
            age: 0,
            center: position,
        },
        OverlayRoot,
        NotUnderTest,
        UiTargetCamera(camera),
        Node {
            left: px(position.x - RING_START_SIZE / 2.0),
            top: px(position.y - RING_START_SIZE / 2.0),
            ..round_marker(RING_START_SIZE, RING_BORDER)
        },
        BorderColor::all(theme.press_ring),
        Pickable::IGNORE,
        GlobalZIndex(OVERLAY_Z_INDEX),
    ));
}

/// Counts frames, not seconds, so every runner draws the same thing.
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
        let center = ring.center;
        node.width = px(size);
        node.height = px(size);
        node.left = px(center.x - size / 2.0);
        node.top = px(center.y - size / 2.0);
        *border = BorderColor::all(
            theme
                .press_ring
                .with_alpha(theme.press_ring.alpha() * (1.0 - progress)),
        );
    }
}

fn touch_mark_bundle(
    kind: TouchMarkKind,
    center: Vec2,
    theme: &OverlayTheme,
    camera: Entity,
) -> impl bevy::ecs::bundle::Bundle {
    let color = match kind {
        TouchMarkKind::Down => theme.touch_down_mark,
        TouchMarkKind::Up => theme.touch_up_mark,
    };
    (
        OverlayTouchMark {
            kind,
            age: 0,
            center,
        },
        OverlayRoot,
        NotUnderTest,
        UiTargetCamera(camera),
        Node {
            left: px(center.x - TOUCH_MARK_START_SIZE / 2.0),
            top: px(center.y - TOUCH_MARK_START_SIZE / 2.0),
            ..round_marker(TOUCH_MARK_START_SIZE, TOUCH_MARK_BORDER)
        },
        BorderColor::all(color),
        Pickable::IGNORE,
        GlobalZIndex(OVERLAY_Z_INDEX),
    )
}

/// Reads Bevy's `Touches`, so anything that writes `TouchInput` looks the same.
fn show_fingers(
    mut commands: Commands,
    touches: Res<Touches>,
    theme: Res<OverlayTheme>,
    aim: Res<OverlayAim>,
    mut fingers: Query<(Entity, &OverlayFinger, &mut Node, &mut BackgroundColor)>,
) {
    let Some(camera) = aim.0 else {
        return;
    };
    for touch in touches.iter_just_pressed() {
        commands.spawn(touch_mark_bundle(
            TouchMarkKind::Down,
            touch.position(),
            &theme,
            camera,
        ));
    }
    for touch in touches
        .iter_just_released()
        .chain(touches.iter_just_canceled())
    {
        commands.spawn(touch_mark_bundle(
            TouchMarkKind::Up,
            touch.position(),
            &theme,
            camera,
        ));
    }
    let mut seen = Vec::new();
    for (entity, OverlayFinger(id), mut node, mut color) in &mut fingers {
        let Some(touch) = touches.get_pressed(*id) else {
            commands.entity(entity).despawn();
            continue;
        };
        seen.push(*id);
        let at = touch.position();
        node.left = px(at.x - FINGER_SIZE / 2.0);
        node.top = px(at.y - FINGER_SIZE / 2.0);
        color.0 = theme.finger;
    }
    for touch in touches.iter() {
        if seen.contains(&touch.id()) {
            continue;
        }
        let at = touch.position();
        commands.spawn((
            OverlayFinger(touch.id()),
            OverlayRoot,
            NotUnderTest,
            UiTargetCamera(camera),
            Node {
                left: px(at.x - FINGER_SIZE / 2.0),
                top: px(at.y - FINGER_SIZE / 2.0),
                ..round_marker(FINGER_SIZE, 0.0)
            },
            BackgroundColor(theme.finger),
            Pickable::IGNORE,
            GlobalZIndex(OVERLAY_Z_INDEX + 1),
        ));
    }
}

/// Counts frames, not seconds, so every runner draws the same thing.
fn animate_touch_marks(
    mut commands: Commands,
    theme: Res<OverlayTheme>,
    mut marks: Query<(Entity, &mut OverlayTouchMark, &mut Node, &mut BorderColor)>,
) {
    for (entity, mut mark, mut node, mut border) in &mut marks {
        // The mark spawned this frame has not been drawn yet.
        mark.age += 1;
        if mark.age >= TOUCH_MARK_FRAMES {
            commands.entity(entity).despawn();
            continue;
        }
        let progress = f32::from(mark.age) / f32::from(TOUCH_MARK_FRAMES);
        let size =
            (TOUCH_MARK_END_SIZE - TOUCH_MARK_START_SIZE).mul_add(progress, TOUCH_MARK_START_SIZE);
        node.width = px(size);
        node.height = px(size);
        node.left = px(mark.center.x - size / 2.0);
        node.top = px(mark.center.y - size / 2.0);
        let color = match mark.kind {
            TouchMarkKind::Down => theme.touch_down_mark,
            TouchMarkKind::Up => theme.touch_up_mark,
        };
        *border = BorderColor::all(color.with_alpha(color.alpha() * (1.0 - progress)));
    }
}

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
    for (dot, mut node, mut color) in &mut dots {
        // Index 0 is where the arrow is now, so the dots start behind it.
        let Some(at) = history.0.get(usize::from(dot.0) + 1) else {
            *color = BackgroundColor(theme.trail.with_alpha(0.0));
            continue;
        };
        let fade = 1.0 - (f32::from(dot.0) / f32::from(TRAIL_LENGTH));
        node.left = px(at.x - TRAIL_DOT_SIZE / 2.0);
        node.top = px(at.y - TRAIL_DOT_SIZE / 2.0);
        *color = BackgroundColor(theme.trail.with_alpha(theme.trail.alpha() * fade));
    }
}

/// Counts frames, not seconds, so every runner draws the same thing.
fn show_checkpoint(
    mut taken: MessageReader<CheckpointTaken>,
    mut notice: ResMut<CheckpointNotice>,
    theme: Res<OverlayTheme>,
    recorder: Option<Res<SessionRecorder>>,
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
    for (mut color, mut visibility) in &mut flash {
        if let Some(age) = fading {
            let left = 1.0 - f32::from(age) / f32::from(CHECKPOINT_FLASH_FRAMES);
            color.0 = theme
                .checkpoint_flash
                .with_alpha(theme.checkpoint_flash.alpha() * left);
            *visibility = Visibility::Inherited;
        } else if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
    }
    // The prompt takes the line for as long as it is open.
    let prompt = recorder.as_deref().and_then(|recorder| {
        let typed = recorder.naming()?;
        Some(naming_text(recorder.checkpoints_taken(), typed))
    });
    let caption = prompt.as_deref().unwrap_or_else(|| {
        notice
            .shown
            .as_ref()
            .map_or("", |(_, caption)| caption.as_str())
    });
    for (mut text, mut color) in &mut line {
        if text.0 != caption {
            caption.clone_into(&mut text.0);
        }
        color.0 = theme.checkpoint_text;
    }
}

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
    for (mut text, mut color) in &mut checks {
        text.0.clone_from(&checks_text);
        color.0 = if failed {
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
