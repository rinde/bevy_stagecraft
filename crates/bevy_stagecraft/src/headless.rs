use core::time::Duration;

use bevy::{
    app::{PluginGroup, PluginGroupBuilder},
    asset::AssetPlugin,
    camera::CameraPlugin,
    gizmos::GizmoPlugin,
    image::{ImagePlugin, TextureAtlasPlugin},
    input::InputPlugin,
    input_focus::{InputDispatchPlugin, InputFocusPlugin},
    math::{UVec2, Vec2},
    mesh::MeshPlugin,
    picking::DefaultPickingPlugins,
    prelude::{App, MinimalPlugins, Plugin, Window, WindowPlugin},
    render::texture::ManualTextureViews,
    text::TextPlugin,
    time::TimeUpdateStrategy,
    transform::TransformPlugin,
    ui::UiPlugin,
    ui_widgets::UiWidgetsPlugins,
    window::{ExitCondition, WindowResolution},
};

use crate::input::SyntheticInputPlugin;

/// Logical pixels.
pub const DEFAULT_WINDOW_SIZE: (u32, u32) = (1280, 720);

pub const DEFAULT_SCALE_FACTOR: f32 = 1.0;

pub const FRAME_STEP: Duration = Duration::from_nanos(1_000_000_000 / 60);

/// Runs a real Bevy app, UI included, with no GPU and no display. Register
/// resources the render plugins would provide yourself; do not silence systems
/// by downgrading the error handler.
#[derive(Debug, Clone, Copy)]
pub struct HeadlessPlugins {
    window_size: (u32, u32),
    scale_factor: f32,
}

impl Default for HeadlessPlugins {
    fn default() -> Self {
        Self {
            window_size: DEFAULT_WINDOW_SIZE,
            scale_factor: DEFAULT_SCALE_FACTOR,
        }
    }
}

impl HeadlessPlugins {
    #[must_use]
    pub const fn with_window_size(mut self, width: u32, height: u32) -> Self {
        self.window_size = (width, height);
        self
    }

    #[must_use]
    pub const fn window_size(&self) -> (u32, u32) {
        self.window_size
    }

    /// Ignores a value that is not finite and above zero.
    #[must_use]
    pub const fn with_scale_factor(mut self, scale_factor: f32) -> Self {
        if scale_factor.is_finite() && scale_factor > 0.0 {
            self.scale_factor = scale_factor;
        }
        self
    }

    #[must_use]
    pub const fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    fn resolution(&self) -> WindowResolution {
        let (width, height) = self.window_size;
        let mut resolution = WindowResolution::new(width, height);
        resolution.set_scale_factor_override(Some(self.scale_factor));
        let physical = (UVec2::new(width, height).as_vec2() * Vec2::splat(self.scale_factor))
            .round()
            .as_uvec2();
        resolution.set_physical_resolution(physical.x, physical.y);
        resolution
    }
}

impl PluginGroup for HeadlessPlugins {
    fn build(self) -> PluginGroupBuilder {
        let resolution = self.resolution();
        PluginGroupBuilder::start::<Self>()
            .add_group(MinimalPlugins)
            .add(AssetPlugin::default())
            .add(WindowPlugin {
                primary_window: Some(Window {
                    resolution,
                    ..Window::default()
                }),
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
                ..WindowPlugin::default()
            })
            .add(InputPlugin)
            .add(TransformPlugin)
            .add(ImagePlugin::default())
            .add(TextureAtlasPlugin)
            .add(MeshPlugin)
            .add(CameraPlugin)
            // Without it UI picking finds no viewport and only hits the window.
            .add(RenderCameraSupport)
            .add(TextPlugin)
            .add(UiPlugin)
            .add_group(DefaultPickingPlugins)
            .add_group(UiWidgetsPlugins)
            .add(InputFocusPlugin)
            .add(InputDispatchPlugin)
            .add(GizmoPlugin)
            .add(FixedTimePlugin)
            .add(SyntheticInputPlugin)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RenderCameraSupport;

impl Plugin for RenderCameraSupport {
    fn build(&self, app: &mut App) {
        app.init_resource::<ManualTextureViews>()
            .add_plugins(bevy::render::camera::CameraPlugin);
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct FixedTimePlugin;

impl Plugin for FixedTimePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(FRAME_STEP));
    }
}
