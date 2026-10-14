//! Checks an app declares itself, as one reflected enum.
//!
//! An app writes its checks as the variants of one enum that derives
//! `Reflect` and carries `#[reflect(AppCheck)]`, and implements [`AppCheck`]
//! for it. Bevy's type registry finds the enum: there is no registration
//! call and no generic parameter. A script writes a value of it inside
//! `App(..)`, which is read against the enum when the script loads, so a
//! misspelt variant or a wrong argument is a load error, not a failure part
//! way through a run.

use core::fmt;

use bevy::reflect::{CreateTypeData, Reflect, TypeRegistry};

use crate::{
    extension::{AppCheckValue, AppValue, CheckPoint, registration},
    probe::Probe,
    scenario::CheckResult,
};

/// A check the app defines, on one reflected enum of its own.
///
/// Derive `Reflect`, add `#[reflect(AppCheck)]` (with [`ReflectAppCheck`] in
/// scope) and implement this trait; Bevy registers the type by itself. One
/// type per app: a second type with `#[reflect(AppCheck)]` is a load error
/// naming both. The app's setups, views and targets follow the same pattern
/// (see [`crate::extension`]).
///
/// Every field of a variant must be `Reflect` as well, since a script's value
/// is read and written through reflection. Primitives, `String` and
/// [`Target`](crate::Target) read best in a file.
///
/// ```
/// use bevy::prelude::*;
/// use rin_bevy_ui_tester::{AppCheck, Check, CheckResult, Probe, ReflectAppCheck};
///
/// #[derive(Resource)]
/// struct Score(u32);
///
/// #[derive(Reflect, Debug, Clone)]
/// #[reflect(AppCheck)]
/// enum GameCheck {
///     /// The score is exactly this.
///     ScoreIs(u32),
/// }
///
/// impl AppCheck for GameCheck {
///     fn run(&self, probe: &Probe) -> CheckResult {
///         match self {
///             Self::ScoreIs(wanted) => match probe.resource::<Score>() {
///                 Some(score) if score.0 == *wanted => Ok(()),
///                 Some(score) => Err(format!("the score is {}", score.0)),
///                 None => Err("there is no score".into()),
///             },
///         }
///     }
/// }
///
/// let check = Check::app(GameCheck::ScoreIs(3));
/// assert_eq!(check.to_string(), "ScoreIs(3)");
/// ```
///
/// # Facts for recordings
///
/// When a recording stops, the checks it suggests (see [`crate::check::suggest`])
/// can include the app's own: implement [`AppCheck::facts`] to list the
/// checks that hold in a world. A check listed at the end of the recording
/// but not at its start is suggested.
pub trait AppCheck {
    /// Whether the check holds in the world `probe` reads.
    ///
    /// # Errors
    ///
    /// What is wrong, when it does not hold.
    fn run(&self, probe: &Probe<'_>) -> CheckResult;

    /// The checks of this type that hold in the world `probe` reads, for a
    /// recording to suggest (see [`crate::check::suggest`]). Keep the list short
    /// and the order stable: it is what the person picks from. None by
    /// default.
    ///
    /// `Self: Sized` keeps the trait usable as `dyn AppCheck`; the registry
    /// reaches this function through [`ReflectAppCheck`].
    #[must_use]
    fn facts(_probe: &Probe<'_>) -> Vec<Self>
    where
        Self: Sized,
    {
        Vec::new()
    }
}

/// What a recording calls to list the app's facts: [`AppCheck::facts`], with
/// each fact boxed as reflection.
type FactsFn = fn(&Probe<'_>) -> Vec<Box<dyn Reflect>>;

/// The type data `#[reflect(AppCheck)]` registers: how to see a reflected
/// value as an [`AppCheck`], and how to list the facts of its type.
///
/// Written by hand rather than with `#[reflect_trait]`, which can only cast a
/// value to the trait: [`AppCheck::facts`] has no value to cast, so its
/// function pointer is kept here, made for the concrete type when Bevy
/// registers it.
#[derive(Clone)]
pub struct ReflectAppCheck {
    /// Casts a value of the registered type to the trait.
    get: fn(&dyn Reflect) -> Option<&dyn AppCheck>,
    /// The registered type's [`AppCheck::facts`].
    facts: FactsFn,
}

impl ReflectAppCheck {
    /// `value` as an [`AppCheck`], when it is of the registered type.
    #[must_use]
    pub fn get<'a>(&self, value: &'a dyn Reflect) -> Option<&'a dyn AppCheck> {
        (self.get)(value)
    }

    /// The registered type's [`AppCheck::facts`] in the world `probe` reads,
    /// each a value of that type.
    #[must_use]
    pub fn facts(&self, probe: &Probe<'_>) -> Vec<Box<dyn Reflect>> {
        (self.facts)(probe)
    }
}

impl<T: AppCheck + Reflect> CreateTypeData<T> for ReflectAppCheck {
    fn create_type_data((): ()) -> Self {
        Self {
            get: |value| {
                value
                    .downcast_ref::<T>()
                    .map(|value| -> &dyn AppCheck { value })
            },
            facts: |probe| {
                T::facts(probe)
                    .into_iter()
                    .map(|fact| -> Box<dyn Reflect> { Box::new(fact) })
                    .collect()
            },
        }
    }
}

// By hand: function pointers say nothing worth printing.
impl fmt::Debug for ReflectAppCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflectAppCheck").finish_non_exhaustive()
    }
}

impl AppValue<CheckPoint> {
    /// Runs the check, reading it as the app's type first if it is not yet.
    /// The outer error means it cannot run at all.
    pub(super) fn run(&self, probe: Probe<'_>) -> Result<CheckResult, String> {
        let typed = self.typed_in(probe.world())?;
        let (value, data) = typed
            .typed_parts()
            .ok_or_else(|| format!("the app check `{self}` has no type"))?;
        let check = data
            .get(value)
            .ok_or_else(|| format!("`{self}` is not an `AppCheck`"))?;
        Ok(check.run(&probe))
    }
}

/// The facts the app's check type lists for the world `probe` reads (see
/// [`AppCheck::facts`]), in its order, without repeats; none when no type, or
/// more than one, is an app check.
pub(super) fn app_facts(types: &TypeRegistry, probe: Probe<'_>) -> Vec<AppCheckValue> {
    let Ok(registration) = registration::<CheckPoint>(types) else {
        return Vec::new();
    };
    let Some(check) = registration.data::<ReflectAppCheck>() else {
        return Vec::new();
    };
    let mut facts: Vec<AppCheckValue> = Vec::new();
    for fact in check.facts(&probe) {
        let value = AppValue::from_typed(types, fact, check.clone());
        if !facts.contains(&value) {
            facts.push(value);
        }
    }
    facts
}
