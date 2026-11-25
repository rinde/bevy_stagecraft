//! Running one scenario in several environments (window sizes, scale factors, text sizes,
//! themes), as Playwright's projects.
//!
//! A [`Variant`] is a runner's setting, not part of a scenario. The default changes nothing; a
//! runner that lists variants covers exactly those, so list the default too.
//!
//! A fixed screen position is in logical pixels, so a variant that sets another window size is
//! an error naming the step, unless the scenario [restricts itself](Scenario::variants).

use core::fmt;

use bevy::reflect::{GetTypeRegistration, Reflect, TypePath};

use crate::{
    extension::{AppSetup, AppView, Setup, View},
    scenario::{Scenario, Step, check_rem_size, flatten, with_group_path},
    script::{LoadError, check_scale_factor},
    targets::Target,
};

/// ```
/// use bevy_stagecraft::{Scenario, Variant};
///
/// let narrow = Variant::new("narrow").window_size(640, 480).scale_factor(2.0);
/// let scenario = Scenario::new("wrap");
/// assert_eq!(narrow.label(scenario.name()), "wrap [narrow]");
/// assert_eq!(Variant::default().label(scenario.name()), "wrap");
/// let varied = narrow.apply(&scenario).unwrap();
/// assert_eq!(varied.window(), Some((640, 480)));
/// assert_eq!(varied.scale(), Some(2.0));
/// ```
#[derive(Debug, Clone, Default)]
pub struct Variant {
    name: String,
    window: Option<(u32, u32)>,
    scale_factor: Option<f32>,
    rem_size: Option<f32>,
    setup: Option<Setup>,
    view: Option<View>,
}

impl Variant {
    /// Give every variant in a list its own name: nothing checks, and equal names make tests,
    /// titles and gallery rows indistinguishable.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub const fn window_size(mut self, width: u32, height: u32) -> Self {
        self.window = Some((width, height));
        self
    }

    /// The window keeps its logical size. Must be finite and above zero, or
    /// [`Variant::apply`] refuses it.
    #[must_use]
    pub const fn scale_factor(mut self, scale_factor: f32) -> Self {
        self.scale_factor = Some(scale_factor);
        self
    }

    /// Sets `RemSize` before the scenario's setup; the runner restores it. Must be finite and
    /// above zero, or [`Variant::apply`] refuses it.
    #[must_use]
    pub const fn rem_size(mut self, rem_size: f32) -> Self {
        self.rem_size = Some(rem_size);
        self
    }

    /// A scenario's own setup closure still runs after it.
    #[must_use]
    pub fn app_setup<T: AppSetup + Reflect + GetTypeRegistration + TypePath>(
        mut self,
        setup: T,
    ) -> Self {
        self.setup = Some(Setup::app(setup));
        self
    }

    /// A scenario's own view closure still runs after it.
    #[must_use]
    pub fn app_view<T: AppView + Reflect + GetTypeRegistration + TypePath>(
        mut self,
        view: T,
    ) -> Self {
        self.view = Some(View::app(view));
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn window(&self) -> Option<(u32, u32)> {
        self.window
    }

    #[must_use]
    pub const fn scale(&self) -> Option<f32> {
        self.scale_factor
    }

    #[must_use]
    pub const fn setup_value(&self) -> Option<&Setup> {
        self.setup.as_ref()
    }

    #[must_use]
    pub const fn view_value(&self) -> Option<&View> {
        self.view.as_ref()
    }

    #[must_use]
    pub const fn is_default(&self) -> bool {
        self.window.is_none()
            && self.scale_factor.is_none()
            && self.setup.is_none()
            && self.view.is_none()
    }

    #[must_use]
    pub fn label(&self, scenario: &str) -> String {
        if self.name.is_empty() {
            scenario.to_owned()
        } else {
            format!("{scenario} [{}]", self.name)
        }
    }

    /// # Errors
    ///
    /// [`LoadError::Invalid`] for a bad scale factor, or when another window size meets a step
    /// that aims at a fixed screen position.
    pub fn apply(&self, scenario: &Scenario) -> Result<Scenario, LoadError> {
        self.problem(scenario)
            .map_or_else(|| Ok(self.varied(scenario)), Err)
    }

    /// Whether `scenario` names the variants it runs in and this is not one. The default
    /// variant answers to `default`.
    #[must_use]
    pub fn skips(&self, scenario: &Scenario) -> bool {
        let names = scenario.variant_names();
        if names.is_empty() {
            return false;
        }
        !names.iter().any(|name| self.matches_name(name))
    }

    fn matches_name(&self, name: &str) -> bool {
        if name.is_empty() || name == DEFAULT_VARIANT_NAME {
            self.name.is_empty()
        } else {
            self.name == name
        }
    }

    pub(crate) fn problem(&self, scenario: &Scenario) -> Option<LoadError> {
        if let Some(scale_factor) = self.scale_factor
            && let Err(message) = check_scale_factor(scale_factor)
        {
            return Some(LoadError::Invalid {
                used_by: format!("variant `{}`", self.name),
                message,
            });
        }
        if let Some(rem_size) = self.rem_size
            && let Err(message) = check_rem_size(rem_size)
        {
            return Some(LoadError::Invalid {
                used_by: format!("variant `{}`", self.name),
                message,
            });
        }
        // A scale factor, rem size, setup or view leaves the logical window alone, so a fixed
        // position stays valid.
        if !self.changes_window(scenario) {
            return None;
        }
        let steps = screen_steps(scenario.script());
        if steps.is_empty() {
            return None;
        }
        Some(LoadError::Invalid {
            used_by: steps_text(&steps),
            message: format!(
                "aims at a fixed screen position (`Screen(..)` or a `MoveBurst(..)` of \
                 positions), which is in logical pixels and only makes sense in the window \
                 the scenario was written for, but `{}` runs it in another window size; aim at \
                 the widget with `Text(..)`, `Role(..)`, `Component(..)` or `Name(..)` instead, \
                 or list the variants the file runs in with `variants: [..]` so the others are \
                 skipped",
                self.label(scenario.name())
            ),
        })
    }

    fn changes_window(&self, scenario: &Scenario) -> bool {
        self.window.is_some() && self.window != scenario.window()
    }

    pub(crate) fn varied(&self, scenario: &Scenario) -> Scenario {
        scenario.in_environment(
            self.label(scenario.name()),
            self.window,
            self.scale_factor,
            self.rem_size,
            self.setup.as_ref(),
            self.view.as_ref(),
        )
    }
}

/// A variant with no name is written as the empty string; this name stands for it in a list.
pub const DEFAULT_VARIANT_NAME: &str = "default";

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.name.is_empty() {
            f.write_str("default")
        } else {
            f.write_str(&self.name)
        }
    }
}

fn screen_steps(steps: &[Step]) -> Vec<(usize, Vec<String>)> {
    flatten(steps)
        .into_iter()
        .enumerate()
        .filter(|(_, flat)| aims_at_screen(flat.step))
        .map(|(index, flat)| (index, flat.group.into_iter().map(str::to_owned).collect()))
        .collect()
}

fn aims_at_screen(step: &Step) -> bool {
    match step {
        Step::MoveBurst(_) => true,
        Step::Timeout(_, inner) | Step::Force(inner) => aims_at_screen(inner),
        other => step_targets(other).into_iter().any(has_screen),
    }
}

fn step_targets(step: &Step) -> Vec<&Target> {
    match step {
        Step::MoveTo(target)
        | Step::Glide { target, .. }
        | Step::TouchDown { at: target, .. }
        | Step::TouchMove { to: target, .. }
        | Step::TouchTap(target) => vec![target],
        Step::Drag { from, to, .. } | Step::TouchDrag { from, to, .. } => vec![from, to],
        Step::Timeout(_, inner) | Step::Force(inner) => step_targets(inner),
        _ => Vec::new(),
    }
}

fn has_screen(target: &Target) -> bool {
    match target {
        Target::Screen(_) => true,
        Target::Within(outer, inner) => has_screen(outer) || has_screen(inner),
        Target::Nth(_, inner) => has_screen(inner),
        _ => false,
    }
}

fn steps_text(steps: &[(usize, Vec<String>)]) -> String {
    if let [(index, group)] = steps {
        return with_group_path(group, &format!("step {index}"));
    }
    let list: Vec<String> = steps
        .iter()
        .map(|(index, group)| with_group_path(group, &index.to_string()))
        .collect();
    format!("steps {}", list.join(", "))
}
