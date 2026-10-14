//! Where a step acts, as the app's own reflected enum.

use core::fmt;

use bevy::{
    ecs::world::World,
    math::Vec2,
    reflect::{CreateTypeData, Reflect},
};

use super::{AppValue, TargetPoint};
use crate::targets::ResolvedTarget;

/// A target the app defines, on one reflected enum of its own: somewhere
/// this crate knows nothing about, such as a station or a lattice point.
///
/// Derive `Reflect`, add `#[reflect(AppTarget)]` (with [`ReflectAppTarget`]
/// in scope) and implement this trait; Bevy registers the type by itself. One
/// type per app: a second type with `#[reflect(AppTarget)]` is a load error
/// naming both. A script writes a value inside `App(..)` wherever a target
/// goes: `MoveTo(App(Station("Alpha")))`.
///
/// ```
/// use bevy::prelude::*;
/// use rin_bevy_ui_tester::{AppTarget, ReflectAppTarget, ResolvedTarget, Step, Target};
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
    /// Works the target out to a pixel in the current state of `world`.
    ///
    /// # Errors
    ///
    /// Why the target cannot be found right now; the step then fails.
    fn resolve(&self, world: &World) -> Result<ResolvedTarget, String>;

    /// The app's own target at `position` (logical pixels in the primary
    /// window), for a click a session recording logs that landed on no
    /// labelled UI node, and for the targets the gallery's pick mode offers;
    /// `None`, the default, keeps the plain screen position.
    ///
    /// `Self: Sized` keeps the trait usable as `dyn AppTarget`; the recorder
    /// reaches this function through [`ReflectAppTarget`].
    #[must_use]
    fn at(_world: &World, _position: Vec2) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}

/// What a recording calls to name a position as the app's target:
/// [`AppTarget::at`], with the target boxed as reflection.
type AtFn = fn(&World, Vec2) -> Option<Box<dyn Reflect>>;

/// The type data `#[reflect(AppTarget)]` registers: how to see a reflected
/// value as an [`AppTarget`], and how to name a position as one of its type.
///
/// Written by hand rather than with `#[reflect_trait]`, which can only cast a
/// value to the trait: [`AppTarget::at`] has no value to cast, so its
/// function pointer is kept here, made for the concrete type when Bevy
/// registers it.
#[derive(Clone)]
pub struct ReflectAppTarget {
    /// Casts a value of the registered type to the trait.
    get: fn(&dyn Reflect) -> Option<&dyn AppTarget>,
    /// The registered type's [`AppTarget::at`].
    at: AtFn,
}

impl ReflectAppTarget {
    /// `value` as an [`AppTarget`], when it is of the registered type.
    #[must_use]
    pub fn get<'a>(&self, value: &'a dyn Reflect) -> Option<&'a dyn AppTarget> {
        (self.get)(value)
    }

    /// The registered type's [`AppTarget::at`].
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

// By hand: function pointers say nothing worth printing.
impl fmt::Debug for ReflectAppTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflectAppTarget").finish_non_exhaustive()
    }
}

impl AppValue<TargetPoint> {
    /// Works the target out to a pixel, reading it as the app's type first
    /// if it is not yet.
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
