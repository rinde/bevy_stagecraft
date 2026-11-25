//! Checks an app declares itself, as one reflected enum.
//!
//! A misspelled variant or a wrong argument is a load error, not a failure
//! part way through a run.

use core::fmt;

use bevy::reflect::{CreateTypeData, Reflect, TypeRegistry};

use crate::{
    extension::{AppCheckValue, AppValue, CheckPoint, registration},
    probe::Probe,
    scenario::CheckResult,
};

/// A check the app defines, on one reflected enum of its own.
///
/// Add `#[reflect(AppCheck)]` to the enum; a second such type is a load error.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_stagecraft::{AppCheck, Check, CheckResult, Probe, ReflectAppCheck};
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
pub trait AppCheck {
    /// # Errors
    ///
    /// The outer error means the check cannot run at all.
    fn run(&self, probe: &Probe<'_>) -> CheckResult;

    /// Checks that hold in the world `probe` reads. Keep the list short and
    /// its order stable: a recording suggests from it.
    #[must_use]
    fn facts(_probe: &Probe<'_>) -> Vec<Self>
    where
        Self: Sized,
    {
        Vec::new()
    }
}

type FactsFn = fn(&Probe<'_>) -> Vec<Box<dyn Reflect>>;

/// Type data for `#[reflect(AppCheck)]`. Written by hand: `#[reflect_trait]`
/// cannot carry [`AppCheck::facts`], which has no value to cast.
#[derive(Clone)]
pub struct ReflectAppCheck {
    get: fn(&dyn Reflect) -> Option<&dyn AppCheck>,
    facts: FactsFn,
}

impl ReflectAppCheck {
    #[must_use]
    pub fn get<'a>(&self, value: &'a dyn Reflect) -> Option<&'a dyn AppCheck> {
        (self.get)(value)
    }

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

impl fmt::Debug for ReflectAppCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflectAppCheck").finish_non_exhaustive()
    }
}

impl AppValue<CheckPoint> {
    /// The outer error means the check cannot run at all.
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

/// The app's facts in its order, without repeats; none unless exactly one type
/// is an app check.
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
