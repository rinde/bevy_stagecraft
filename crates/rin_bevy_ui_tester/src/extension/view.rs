//! How the world is shown, as the app's own reflected enum.

use core::fmt;

use bevy::{
    ecs::world::World,
    reflect::{CreateTypeData, GetTypeRegistration, Reflect, TypePath, TypeRegistry},
};
use serde::{Deserialize, Serialize, de::Error as _};

use super::{AppValue, AppViewValue, ViewPoint};
use crate::check::Problem;

/// A view the app defines, on one reflected enum of its own: what arranges
/// how the world is shown (the camera, the layout).
///
/// Derive `Reflect`, add `#[reflect(AppView)]` (with [`ReflectAppView`] in
/// scope) and implement this trait; Bevy registers the type by itself. One
/// type per app: a second type with `#[reflect(AppView)]` is a load error
/// naming both. A script writes a value as `view: App(..)`. Headless runs do
/// not arrange a view; the recorder and the gallery do, after the setup.
///
/// ```
/// use bevy::prelude::*;
/// use rin_bevy_ui_tester::{AppView, ReflectAppView, Scenario};
///
/// #[derive(Reflect, Debug, Clone)]
/// #[reflect(AppView)]
/// enum GameView {
///     /// Seen from straight above.
///     TopDown,
/// }
///
/// impl AppView for GameView {
///     fn arrange(&self, world: &mut World) {
///         world.spawn(Camera2d);
///     }
/// }
///
/// let scenario = Scenario::new("from above").app_view(GameView::TopDown);
/// assert!(scenario.to_ron().unwrap().contains("view: App(TopDown)"));
/// ```
pub trait AppView {
    /// Arranges how `world` is shown.
    fn arrange(&self, world: &mut World);

    /// The view `world` is shown in now, for a session recording that starts
    /// here; `None`, the default, records no view.
    ///
    /// `Self: Sized` keeps the trait usable as `dyn AppView`; the recorder
    /// reaches this function through [`ReflectAppView`].
    #[must_use]
    fn current(_world: &World) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}

/// What a recording calls to name the app's view: [`AppView::current`],
/// with the view boxed as reflection.
type CurrentFn = fn(&World) -> Option<Box<dyn Reflect>>;

/// The type data `#[reflect(AppView)]` registers: how to see a reflected
/// value as an [`AppView`], and how to find the current one of its type.
///
/// Written by hand rather than with `#[reflect_trait]`, which can only cast a
/// value to the trait: [`AppView::current`] has no value to cast, so its
/// function pointer is kept here, made for the concrete type when Bevy
/// registers it.
#[derive(Clone)]
pub struct ReflectAppView {
    /// Casts a value of the registered type to the trait.
    get: fn(&dyn Reflect) -> Option<&dyn AppView>,
    /// The registered type's [`AppView::current`].
    current: CurrentFn,
}

impl ReflectAppView {
    /// `value` as an [`AppView`], when it is of the registered type.
    #[must_use]
    pub fn get<'a>(&self, value: &'a dyn Reflect) -> Option<&'a dyn AppView> {
        (self.get)(value)
    }

    /// The registered type's [`AppView::current`].
    #[must_use]
    pub fn current(&self, world: &World) -> Option<Box<dyn Reflect>> {
        (self.current)(world)
    }
}

impl<T: AppView + Reflect> CreateTypeData<T> for ReflectAppView {
    fn create_type_data((): ()) -> Self {
        Self {
            get: |value| {
                value
                    .downcast_ref::<T>()
                    .map(|value| -> &dyn AppView { value })
            },
            current: |world| T::current(world).map(|view| -> Box<dyn Reflect> { Box::new(view) }),
        }
    }
}

// By hand: function pointers say nothing worth printing.
impl fmt::Debug for ReflectAppView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflectAppView").finish_non_exhaustive()
    }
}

/// What a file that refers to a view by name, as format versions 1 to 3
/// could, is told.
const NO_NAMED_VIEW: &str = "views are no longer looked up by name: declare the \
    app's views as one enum with `#[derive(Reflect)]` and `#[reflect(AppView)]`, and write one \
    as `App(..)`, such as `view: App(TopDown)`";

/// How a scenario's world is shown, as a script holds it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub enum View {
    /// One of the app's own views: a value of the type the app marks with
    /// `#[reflect(AppView)]` (see [`AppView`]). Written as `App(TopDown)`.
    App(AppViewValue),
}

/// [`View`] as serde's derive reads it, which [`View`]'s own impl adds the
/// hint for the old form to.
#[derive(Deserialize)]
#[serde(remote = "View", rename = "View")]
enum Written {
    /// See [`View::App`].
    App(AppViewValue),
}

/// Any form but `App(..)`, such as the old bare name, is refused saying what
/// to write instead.
impl<'de> Deserialize<'de> for View {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Written::deserialize(deserializer)
            .map_err(|error| D::Error::custom(format!("{error}: {NO_NAMED_VIEW}")))
    }
}

impl View {
    /// One of the app's own views, for example `View::app(RailView::TopDown)`.
    #[must_use]
    pub fn app<T: AppView + Reflect + GetTypeRegistration + TypePath>(view: T) -> Self {
        Self::App(AppValue::of(view))
    }

    /// Arranges the view in `world`.
    pub(crate) fn arrange(&self, world: &mut World) -> Result<(), String> {
        match self {
            Self::App(value) => value.arrange(world),
        }
    }

    /// Reads the app's value as its view type in `types`.
    pub(crate) fn type_against(&mut self, types: &TypeRegistry) -> Result<(), Problem> {
        match self {
            Self::App(value) => value.type_against(types),
        }
    }
}

impl AppValue<ViewPoint> {
    /// Arranges the view in `world`, reading it as the app's type first if
    /// it is not yet.
    pub(crate) fn arrange(&self, world: &mut World) -> Result<(), String> {
        let typed = self.typed_in(world)?.into_owned();
        let (value, data) = typed
            .typed_parts()
            .ok_or_else(|| format!("the app view `{self}` has no type"))?;
        let view = data
            .get(value)
            .ok_or_else(|| format!("`{self}` is not an `AppView`"))?;
        view.arrange(world);
        Ok(())
    }
}
