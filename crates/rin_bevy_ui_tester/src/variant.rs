//! Running one scenario in several environments: window sizes, scale
//! factors, text sizes, themes.
//!
//! A [`Variant`] is a runner's setting, not part of a scenario: the same file
//! is played once per variant the runner lists, which is what Playwright calls
//! projects. A variant sets the window size and scale factor, and may replace
//! the scenario's app setup or view (for example to switch the theme).
//!
//! * [`crate::run_ron_tests`] with variants makes one test per scenario and
//!   variant, named `scenario [variant]`.
//! * The recorder writes one index entry per scenario and variant.
//! * The gallery has a variant picker.
//!
//! The variant a runner starts from is [`Variant::default`]: it changes
//! nothing, so the scenario runs as written. A runner that lists variants
//! covers exactly those, so list the default one too to cover the scenario as
//! it is written.
//!
//! A step that aims at a fixed screen position ([`Target::Screen`]) is in
//! logical pixels, so it only makes sense where the logical window is the one
//! the position was written for. A variant that only changes the scale factor
//! or rem size, or the app's setup or view, leaves that window alone and runs
//! the step; a
//! variant that sets another window size is an error naming the step unless
//! the scenario [restricts itself](Scenario::variants) to the variants it is
//! valid in, where the others are skipped rather than failed.

use core::fmt;

use bevy::reflect::{GetTypeRegistration, Reflect, TypePath};

use crate::{
    extension::{AppSetup, AppView, Setup, View},
    scenario::{Scenario, Step, check_rem_size, flatten, with_group_path},
    script::{LoadError, check_scale_factor},
    targets::Target,
};

/// One environment to run scenarios in. See the [module](self).
///
/// ```
/// use rin_bevy_ui_tester::{Scenario, Variant};
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
    /// What the variant is called; empty for the default one.
    name: String,
    /// The window size to run in, in logical pixels, instead of the
    /// scenario's.
    window: Option<(u32, u32)>,
    /// The scale factor to run at, instead of the scenario's.
    scale_factor: Option<f32>,
    /// The root font size (Bevy's `RemSize`) to run at, in logical pixels.
    rem_size: Option<f32>,
    /// The app's setup to run instead of the scenario's.
    setup: Option<Setup>,
    /// The app's view to arrange instead of the scenario's.
    view: Option<View>,
}

impl Variant {
    /// A variant called `name` that changes nothing yet; add what it changes
    /// with [`Variant::window_size`], [`Variant::scale_factor`],
    /// [`Variant::app_setup`] and [`Variant::app_view`].
    ///
    /// Give every variant in a list its own name. The name is part of the
    /// test name, the index title and the gallery row (`scenario [name]`), and
    /// nothing checks that two variants differ: with the same name, their
    /// tests, titles and rows cannot be told apart. Only the recorder keeps
    /// their folders apart.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Runs in a window of this size, in logical pixels.
    #[must_use]
    pub const fn window_size(mut self, width: u32, height: u32) -> Self {
        self.window = Some((width, height));
        self
    }

    /// Runs at this scale factor: physical pixels per logical pixel, as on a
    /// high-density display. The window keeps its logical size. It must be
    /// finite and above zero, or [`Variant::apply`] refuses it.
    #[must_use]
    pub const fn scale_factor(mut self, scale_factor: f32) -> Self {
        self.scale_factor = Some(scale_factor);
        self
    }

    /// Runs with this root font size, in logical pixels, set on the app's
    /// `RemSize` before the scenario's setup (Bevy's default is 20), so text
    /// and layout in `Rem` units grow or shrink as with a larger or smaller
    /// text size setting. The window keeps its logical size, so a `Screen`
    /// target or a `MoveBurst` stays valid. It must be finite and above zero,
    /// or [`Variant::apply`] refuses it. The runner puts the size back where
    /// it changed it; text the app sizes in `Px` does not follow it.
    #[must_use]
    pub const fn rem_size(mut self, rem_size: f32) -> Self {
        self.rem_size = Some(rem_size);
        self
    }

    /// Runs this setup of the app, a value of its `#[reflect(AppSetup)]` enum,
    /// instead of the scenario's own (a scenario's setup closure still runs
    /// after it). The way to switch a theme the app's setup decides.
    #[must_use]
    pub fn app_setup<T: AppSetup + Reflect + GetTypeRegistration + TypePath>(
        mut self,
        setup: T,
    ) -> Self {
        self.setup = Some(Setup::app(setup));
        self
    }

    /// Arranges this view of the app, a value of its `#[reflect(AppView)]`
    /// enum, instead of the scenario's own (a scenario's view closure still
    /// runs after it).
    #[must_use]
    pub fn app_view<T: AppView + Reflect + GetTypeRegistration + TypePath>(
        mut self,
        view: T,
    ) -> Self {
        self.view = Some(View::app(view));
        self
    }

    /// What the variant is called; empty for the default one.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The window size it runs in, if it sets one.
    #[must_use]
    pub const fn window(&self) -> Option<(u32, u32)> {
        self.window
    }

    /// The scale factor it runs at, if it sets one.
    #[must_use]
    pub const fn scale(&self) -> Option<f32> {
        self.scale_factor
    }

    /// The app's setup it runs instead of the scenario's, if it sets one.
    #[must_use]
    pub const fn setup_value(&self) -> Option<&Setup> {
        self.setup.as_ref()
    }

    /// The app's view it arranges instead of the scenario's, if it sets one.
    #[must_use]
    pub const fn view_value(&self) -> Option<&View> {
        self.view.as_ref()
    }

    /// Whether this is the default variant: it changes nothing, so a scenario
    /// runs as written.
    #[must_use]
    pub const fn is_default(&self) -> bool {
        self.window.is_none()
            && self.scale_factor.is_none()
            && self.setup.is_none()
            && self.view.is_none()
    }

    /// What a run of the scenario called `scenario` in this variant is called:
    /// `scenario [variant]`, or just `scenario` in the default variant. It is
    /// the name of a test, an index entry and a gallery row.
    #[must_use]
    pub fn label(&self, scenario: &str) -> String {
        if self.name.is_empty() {
            scenario.to_owned()
        } else {
            format!("{scenario} [{}]", self.name)
        }
    }

    /// The scenario as this variant runs it: a copy called
    /// [`Variant::label`] whose window size, scale factor, app setup and app
    /// view are the variant's where it sets them. The steps and checks are the
    /// scenario's own.
    ///
    /// # Errors
    ///
    /// [`LoadError::Invalid`] when the scale factor is not finite and above
    /// zero, or when this variant runs the scenario in another window size and
    /// a step aims at a fixed screen position ([`Target::Screen`] or a
    /// [`Step::MoveBurst`], whose positions are logical too). It names the
    /// steps, counted from 0 as a file's loader does.
    pub fn apply(&self, scenario: &Scenario) -> Result<Scenario, LoadError> {
        self.problem(scenario)
            .map_or_else(|| Ok(self.varied(scenario)), Err)
    }

    /// Whether `scenario` restricts itself away from this variant: it names
    /// the variants it runs in ([`Scenario::variant_names`], a file's
    /// `variants: [..]`) and this is not one of them. A runner skips such a
    /// file and variant rather than failing it. A scenario that names none
    /// runs in every variant.
    ///
    /// The default variant answers to the name `default` (the empty name a
    /// variant with no name has).
    #[must_use]
    pub fn skips(&self, scenario: &Scenario) -> bool {
        let names = scenario.variant_names();
        if names.is_empty() {
            return false;
        }
        !names.iter().any(|name| self.matches_name(name))
    }

    /// Whether this variant answers to `name`: its own name, or `default`
    /// (or the empty string) for a variant with no name.
    fn matches_name(&self, name: &str) -> bool {
        if name.is_empty() || name == DEFAULT_VARIANT_NAME {
            self.name.is_empty()
        } else {
            self.name == name
        }
    }

    /// Why `scenario` cannot run in this variant, if it cannot; see
    /// [`Variant::apply`].
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
        // A fixed position is in logical pixels, so it stays valid wherever
        // the logical window is the one it was written for: a scale factor, a
        // rem size, a setup or a view leaves that window alone.
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

    /// Whether this variant runs `scenario` in a different logical window
    /// than the scenario's own. A window size the scenario already asks for
    /// changes nothing.
    fn changes_window(&self, scenario: &Scenario) -> bool {
        self.window.is_some() && self.window != scenario.window()
    }

    /// The scenario as this variant runs it, whether or not it can.
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

/// The name the default variant answers to in a scenario's variant list
/// ([`Scenario::variant_names`]): `default`, because a variant with no name
/// is written as the empty string.
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

/// The steps that aim the pointer at a screen position, as the report
/// counts them (from 0, groups not counted), with the groups around each.
fn screen_steps(steps: &[Step]) -> Vec<(usize, Vec<String>)> {
    flatten(steps)
        .into_iter()
        .enumerate()
        .filter(|(_, flat)| aims_at_screen(flat.step))
        .map(|(index, flat)| (index, flat.group.into_iter().map(str::to_owned).collect()))
        .collect()
}

/// Whether `step` aims the pointer at a fixed position of the window: a
/// [`Target::Screen`], or a [`Step::MoveBurst`], whose positions are just as
/// fixed.
fn aims_at_screen(step: &Step) -> bool {
    match step {
        Step::MoveBurst(_) => true,
        Step::Timeout(_, inner) | Step::Force(inner) => aims_at_screen(inner),
        other => step_targets(other).into_iter().any(has_screen),
    }
}

/// The targets a step aims at, inside any `Timeout(..)` or `Force(..)`.
fn step_targets(step: &Step) -> Vec<&Target> {
    match step {
        Step::MoveTo(target) | Step::Glide { target, .. } => vec![target],
        Step::Drag { from, to, .. } => vec![from, to],
        Step::Timeout(_, inner) | Step::Force(inner) => step_targets(inner),
        _ => Vec::new(),
    }
}

/// Whether `target` is, or is built on, a screen position.
fn has_screen(target: &Target) -> bool {
    match target {
        Target::Screen(_) => true,
        Target::Within(outer, inner) => has_screen(outer) || has_screen(inner),
        Target::Nth(_, inner) => has_screen(inner),
        _ => false,
    }
}

/// `step 3`, or `steps 3, 5` for several, each with the groups around it in
/// front (`raise the bridge > step 3`).
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
