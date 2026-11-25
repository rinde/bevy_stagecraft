use core::fmt;

use bevy::{
    ecs::world::World,
    math::Vec2,
    reflect::{CreateTypeData, Reflect},
};

use super::{AppValue, TargetPoint};
use crate::targets::ResolvedTarget;

/// A target the app defines, on one reflected enum marked `#[reflect(AppTarget)]`.
///
/// One type per app: a second one is a load error naming both.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_stagecraft::{AppTarget, ReflectAppTarget, ResolvedTarget, Step, Target};
///
/// #[derive(Reflect, Debug, Clone)]
/// #[reflect(AppTarget)]
/// enum GameTarget {
///     /// A cell of the board, counted from the top left.
///     Cell(u32, u32),
/// }
///
/// impl AppTarget for GameTarget {
///     fn resolve(&self, _world: &World) -> Result<ResolvedTarget, String> {
///         match self {
///             Self::Cell(x, y) => Ok(ResolvedTarget::at(Vec2::new(
///                 20.0 + 40.0 * *x as f32,
///                 20.0 + 40.0 * *y as f32,
///             ))),
///         }
///     }
/// }
///
/// let step = Step::move_to(Target::app(GameTarget::Cell(2, 3)));
/// assert_eq!(format!("{step:?}"), "MoveTo(App(Cell(2,3)))");
/// ```
pub trait AppTarget {
    /// # Errors
    ///
    /// Why the target cannot be found right now; the step then fails.
    fn resolve(&self, world: &World) -> Result<ResolvedTarget, String>;

    /// The app's own target at `position`, for a click a session recording logs that landed
    /// on no labeled UI node and for the gallery's pick mode; `None` keeps the screen position.
    ///
    /// `Self: Sized` keeps the trait usable as `dyn AppTarget`.
    #[must_use]
    fn at(_world: &World, _position: Vec2) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}

type AtFn = fn(&World, Vec2) -> Option<Box<dyn Reflect>>;

/// The type data `#[reflect(AppTarget)]` registers.
///
/// By hand: `#[reflect_trait]` can only cast a value, and [`AppTarget::at`] has no value.
#[derive(Clone)]
pub struct ReflectAppTarget {
    get: fn(&dyn Reflect) -> Option<&dyn AppTarget>,
    at: AtFn,
}

impl ReflectAppTarget {
    #[must_use]
    pub fn get<'a>(&self, value: &'a dyn Reflect) -> Option<&'a dyn AppTarget> {
        (self.get)(value)
    }

    #[must_use]
    pub fn at(&self, world: &World, position: Vec2) -> Option<Box<dyn Reflect>> {
        (self.at)(world, position)
    }
}

impl<T: AppTarget + Reflect> CreateTypeData<T> for ReflectAppTarget {
    fn create_type_data((): ()) -> Self {
        Self {
            get: |value| {
                value
                    .downcast_ref::<T>()
                    .map(|value| -> &dyn AppTarget { value })
            },
            at: |world, position| {
                T::at(world, position).map(|target| -> Box<dyn Reflect> { Box::new(target) })
            },
        }
    }
}

impl fmt::Debug for ReflectAppTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflectAppTarget").finish_non_exhaustive()
    }
}

impl AppValue<TargetPoint> {
    pub(crate) fn resolve(&self, world: &World) -> Result<ResolvedTarget, String> {
        let typed = self.typed_in(world)?;
        let (value, data) = typed
            .typed_parts()
            .ok_or_else(|| format!("the app target `{self}` has no type"))?;
        let target = data
            .get(value)
            .ok_or_else(|| format!("`{self}` is not an `AppTarget`"))?;
        target.resolve(world)
    }
}
