use core::fmt;

use bevy::{
    ecs::world::World,
    reflect::{CreateTypeData, GetTypeRegistration, Reflect, TypePath, TypeRegistry},
};
use serde::{Deserialize, Serialize, de::Error as _};

use super::{AppValue, AppViewValue, ViewPoint};
use crate::check::Problem;

/// A view the app defines, on one reflected enum marked `#[reflect(AppView)]`.
///
/// One type per app: a second one is a load error naming both.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_stagecraft::{AppView, ReflectAppView, Scenario};
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

    /// The view `world` is shown in now, for a session recording; `None` records no view.
    ///
    /// `Self: Sized` keeps the trait usable as `dyn AppView`.
    #[must_use]
    fn current(_world: &World) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}

type CurrentFn = fn(&World) -> Option<Box<dyn Reflect>>;

/// The type data `#[reflect(AppView)]` registers.
///
/// Written by hand: `#[reflect_trait]` can only cast a value, and [`AppView::current`] has
/// no value.
#[derive(Clone)]
pub struct ReflectAppView {
    get: fn(&dyn Reflect) -> Option<&dyn AppView>,
    current: CurrentFn,
}

impl ReflectAppView {
    #[must_use]
    pub fn get<'a>(&self, value: &'a dyn Reflect) -> Option<&'a dyn AppView> {
        (self.get)(value)
    }

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

impl fmt::Debug for ReflectAppView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflectAppView").finish_non_exhaustive()
    }
}

const NO_NAMED_VIEW: &str = "views are no longer looked up by name: declare the \
    app's views as one enum with `#[derive(Reflect)]` and `#[reflect(AppView)]`, and write one \
    as `App(..)`, such as `view: App(TopDown)`";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub enum View {
    App(AppViewValue),
}

#[derive(Deserialize)]
#[serde(remote = "View", rename = "View")]
enum Written {
    App(AppViewValue),
}

impl<'de> Deserialize<'de> for View {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Written::deserialize(deserializer)
            .map_err(|error| D::Error::custom(format!("{error}: {NO_NAMED_VIEW}")))
    }
}

impl View {
    #[must_use]
    pub fn app<T: AppView + Reflect + GetTypeRegistration + TypePath>(view: T) -> Self {
        Self::App(AppValue::of(view))
    }

    pub(crate) fn arrange(&self, world: &mut World) -> Result<(), String> {
        match self {
            Self::App(value) => value.arrange(world),
        }
    }

    pub(crate) fn type_against(&mut self, types: &TypeRegistry) -> Result<(), Problem> {
        match self {
            Self::App(value) => value.type_against(types),
        }
    }
}

impl AppValue<ViewPoint> {
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
