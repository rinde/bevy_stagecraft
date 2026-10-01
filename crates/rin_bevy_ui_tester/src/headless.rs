//! Running a real Bevy app, UI included, with no GPU and no display.

use core::time::Duration;

use bevy::{
    app::{PluginGroup, PluginGroupBuilder},
    asset::AssetPlugin,
    camera::CameraPlugin,
    gizmos::GizmoPlugin,
    image::{ImagePlugin, TextureAtlasPlugin},
    input::InputPlugin,
    input_focus::{InputDispatchPlugin, InputFocusPlugin},
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

/// The size of the virtual primary window unless configured otherwise.
pub const DEFAULT_WINDOW_SIZE: (u32, u32) = (1280, 720);

/// Every frame advances time by exactly this much, in every runner.
pub const FRAME_STEP: Duration = Duration::from_nanos(1_000_000_000 / 60);

/// A plugin group that runs a real Bevy app, UI included, with no GPU and no
/// display.
///
/// It is built on `MinimalPlugins`. The primary window is virtual: there is no
/// winit and no surface, only a `Window` entity of the configured size. Time
/// advances by a fixed [`FRAME_STEP`] per update, so runs are deterministic.
///
/// An app whose own plugins expect resources or asset types that the render
/// plugins normally provide registers those itself, for example with a small
/// support plugin. Do not silence systems by downgrading the error handler.
#[derive(Debug, Clone, Copy)]
pub struct HeadlessPlugins {
    /// Size of the virtual primary window, in logical pixels.
    window_size: (u32, u32),
}

impl Default for HeadlessPlugins {
    fn default() -> Self {
        Self {
            window_size: DEFAULT_WINDOW_SIZE,
        }
    }
}

impl HeadlessPlugins {
    /// Sets the size of the virtual primary window, in logical pixels.
    #[must_use]
    pub const fn with_window_size(mut self, width: u32, height: u32) -> Self {
        self.window_size = (width, height);
        self
    }

    /// The size of the virtual primary window, in logical pixels.
    #[must_use]
    pub const fn window_size(&self) -> (u32, u32) {
        self.window_size
    }
}

impl PluginGroup for HeadlessPlugins {
    fn build(self) -> PluginGroupBuilder {
        let (width, height) = self.window_size;
        PluginGroupBuilder::start::<Self>()
            .add_group(MinimalPlugins)
            .add(AssetPlugin::default())
            .add(WindowPlugin {
                primary_window: Some(Window {
                    resolution: WindowResolution::new(width, height),
                    ..Window::default()
                }),
                // Nothing closes a virtual window, and the app must outlive it.
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
            // The render crate's camera plugin is the only thing that works
            // out a camera's target size. Without it UI picking finds no
            // viewport and only ever hits the window.
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

/// Adds `bevy::render::camera::CameraPlugin` and the resource it reads.
#[derive(Debug, Default, Clone, Copy)]
struct RenderCameraSupport;

impl Plugin for RenderCameraSupport {
    fn build(&self, app: &mut App) {
        app.init_resource::<ManualTextureViews>()
            .add_plugins(bevy::render::camera::CameraPlugin);
    }
}

/// Makes every update advance time by exactly [`FRAME_STEP`].
#[derive(Debug, Default, Clone, Copy)]
struct FixedTimePlugin;

impl Plugin for FixedTimePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(FRAME_STEP));
    }
}
