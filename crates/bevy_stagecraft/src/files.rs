//! Call [`run_ron_tests`] from the `main` of a `harness = false` test target:
//!
//! ```toml
//! [[test]]
//! name = "ron_scenarios"
//! harness = false
//! ```
//!
//! ```ignore
//! fn main() -> std::process::ExitCode {
//!     bevy_stagecraft::run_ron_tests("tests/scenarios", &registry(), |scenario| {
//!         let mut app = scenario.headless_app();
//!         registry().install(app.world_mut());
//!         scenario.run_in(&mut app)
//!     })
//! }
//! ```

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use crate::{
    annotation::{Disabled, TagFilter},
    player::ScenarioReport,
    save::is_scene_file,
    scenario::Scenario,
    script::{LoadError, Registry},
    variant::Variant,
};

#[derive(Debug, Clone, PartialEq)]
pub enum FileOutcome {
    /// `Err` is why it did not load, or why the variant refused it.
    Ran(Result<ScenarioReport, LoadError>),
    Skipped,
    Disabled(Disabled),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileResult {
    pub name: String,
    pub variant: String,
    pub path: PathBuf,
    pub outcome: FileOutcome,
    pub should_fail: Option<String>,
}

impl FileResult {
    /// A skipped file counts as passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        match &self.outcome {
            FileOutcome::Skipped | FileOutcome::Disabled(_) => true,
            FileOutcome::Ran(Ok(report)) => report.passed() != self.should_fail.is_some(),
            FileOutcome::Ran(Err(_)) => false,
        }
    }

    #[must_use]
    pub const fn skipped(&self) -> bool {
        matches!(self.outcome, FileOutcome::Skipped)
    }

    #[must_use]
    pub const fn ignored(&self) -> bool {
        matches!(self.outcome, FileOutcome::Disabled(_))
    }

    #[must_use]
    pub const fn disabled(&self) -> Option<&Disabled> {
        match &self.outcome {
            FileOutcome::Disabled(disabled) => Some(disabled),
            _ => None,
        }
    }

    #[must_use]
    pub fn failed_as_expected(&self) -> bool {
        self.should_fail.is_some()
            && matches!(&self.outcome, FileOutcome::Ran(Ok(report)) if !report.passed())
    }

    #[must_use]
    pub fn failures(&self) -> Vec<String> {
        match &self.outcome {
            FileOutcome::Skipped | FileOutcome::Disabled(_) => Vec::new(),
            FileOutcome::Ran(Ok(report)) if report.passed() => {
                self.should_fail.iter().map(|reason| format!(
                    "expected to fail ({reason}) but passed: the bug is fixed, remove should_fail"
                )).collect()
            }
            FileOutcome::Ran(Ok(_)) if self.should_fail.is_some() => Vec::new(),
            FileOutcome::Ran(Ok(report)) => {
                let mut failures = report.failures();
                if failures.is_empty() {
                    failures.push("the scenario did not finish".into());
                }
                failures
            }
            FileOutcome::Ran(Err(error)) => vec![error.to_string()],
        }
    }
}

/// Skips `*.scene.ron`: saved worlds are setups, not scenarios.
///
/// # Errors
///
/// [`LoadError::Io`] if the directory cannot be read.
pub fn ron_files(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>, LoadError> {
    let dir = dir.as_ref();
    let io = |error: std::io::Error| LoadError::Io {
        path: dir.display().to_string(),
        message: error.to_string(),
    };
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        let is_scene = path
            .file_name()
            .is_some_and(|name| is_scene_file(&name.to_string_lossy()));
        if path.extension().is_some_and(|extension| extension == "ron") && !is_scene {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// A setup that names a file looks for it next to this one.
///
/// # Errors
///
/// [`LoadError`] if it cannot be read or does not load.
pub fn load_ron_file(path: impl AsRef<Path>, registry: &Registry) -> Result<Scenario, LoadError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|error| LoadError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    let scenario = Scenario::from_ron_with(&text, registry)?;
    Ok(match path.parent() {
        Some(dir) => scenario.with_base_dir(dir),
        None => scenario,
    })
}

/// Plays the default variant only.
///
/// # Errors
///
/// [`LoadError::Io`] if the directory cannot be read. A file that does not
/// load is a failed [`FileResult`], not an error here.
pub fn run_ron_files(
    dir: impl AsRef<Path>,
    registry: &Registry,
    filter: &str,
    run: impl FnMut(&Scenario) -> ScenarioReport,
) -> Result<Vec<FileResult>, LoadError> {
    run_ron_variants(dir, registry, filter, &[Variant::default()], run)
}

/// Include [`Variant::default`] to play the file as written too. A file that
/// names the variants it runs in is skipped in the others.
///
/// # Errors
///
/// [`LoadError::Io`] if the directory cannot be read. A file that does not
/// load is a failed [`FileResult`] per variant, not an error here.
pub fn run_ron_variants(
    dir: impl AsRef<Path>,
    registry: &Registry,
    filter: &str,
    variants: &[Variant],
    run: impl FnMut(&Scenario) -> ScenarioReport,
) -> Result<Vec<FileResult>, LoadError> {
    run_ron_tagged(dir, registry, filter, &TagFilter::default(), variants, run)
}

/// A file that does not load is always kept, so its error shows.
///
/// # Errors
///
/// [`LoadError::Io`] if the directory cannot be read.
pub fn run_ron_tagged(
    dir: impl AsRef<Path>,
    registry: &Registry,
    filter: &str,
    tags: &TagFilter,
    variants: &[Variant],
    mut run: impl FnMut(&Scenario) -> ScenarioReport,
) -> Result<Vec<FileResult>, LoadError> {
    let mut results = Vec::new();
    for path in ron_files(dir)? {
        let Some(stem) = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
        else {
            continue;
        };
        let wanted: Vec<(&Variant, String)> = variants
            .iter()
            .map(|variant| (variant, variant.label(&stem)))
            .filter(|(_, name)| name.contains(filter))
            .collect();
        // A file the filter leaves nothing of may be broken; do not read it.
        if wanted.is_empty() {
            continue;
        }
        let loaded = load_ron_file(&path, registry);
        if loaded
            .as_ref()
            .is_ok_and(|scenario| !tags.matches(scenario.tag_names()))
        {
            continue;
        }
        let should_fail = loaded
            .as_ref()
            .ok()
            .and_then(|scenario| scenario.should_fail_reason().map(str::to_owned));
        for (variant, name) in wanted {
            let outcome = match &loaded {
                Ok(scenario) if scenario.disabled().is_some() => FileOutcome::Disabled(
                    scenario.disabled().unwrap_or(Disabled::Skip(String::new())),
                ),
                Ok(scenario) if variant.skips(scenario) => FileOutcome::Skipped,
                Ok(scenario) => {
                    FileOutcome::Ran(variant.apply(scenario).map(|scenario| run(&scenario)))
                }
                Err(error) => FileOutcome::Ran(Err(error.clone())),
            };
            results.push(FileResult {
                name,
                variant: variant.name().to_owned(),
                path: path.clone(),
                outcome,
                should_fail: should_fail.clone(),
            });
        }
    }
    Ok(results)
}

/// Command-line words that do not start with `-` filter by file name.
pub fn run_ron_tests(
    dir: impl AsRef<Path>,
    registry: &Registry,
    run: impl FnMut(&Scenario) -> ScenarioReport,
) -> ExitCode {
    run_ron_tests_with_variants(dir, registry, &[Variant::default()], run)
}

/// A filter word matches `file [variant]`, so `cargo test narrow` plays the
/// narrow variant of every file.
pub fn run_ron_tests_with_variants(
    dir: impl AsRef<Path>,
    registry: &Registry,
    variants: &[Variant],
    run: impl FnMut(&Scenario) -> ScenarioReport,
) -> ExitCode {
    let filter = std::env::args()
        .skip(1)
        .find(|argument| !argument.starts_with('-'))
        .unwrap_or_default();
    let all_passed = run_and_print(
        dir.as_ref(),
        registry,
        &filter,
        &TagFilter::from_env(),
        variants,
        run,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    if all_passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run_and_print(
    dir: &Path,
    registry: &Registry,
    filter: &str,
    tags: &TagFilter,
    variants: &[Variant],
    run: impl FnMut(&Scenario) -> ScenarioReport,
    out: &mut impl Write,
    err: &mut impl Write,
) -> bool {
    match run_ron_tagged(dir, registry, filter, tags, variants, run) {
        Ok(results) => print_results(out, &results),
        Err(error) => {
            // A closed pipe is not worth failing over.
            writeln!(err, "{error}").ok();
            false
        }
    }
}

fn verdict(result: &FileResult) -> String {
    match result.disabled() {
        Some(disabled) => format!("ignored, {}", disabled.describe()),
        None if result.skipped() => "skipped".to_owned(),
        None if result.failed_as_expected() => "ok (failed as expected)".to_owned(),
        None if result.passed() => "ok".to_owned(),
        None => "FAILED".to_owned(),
    }
}

fn print_results(out: &mut impl Write, results: &[FileResult]) -> bool {
    // A closed pipe is not worth failing over.
    let mut say = |line: String| writeln!(out, "{line}").ok();
    say(format!("\nrunning {} tests", results.len()));
    for result in results {
        let verdict = verdict(result);
        say(format!("test {} ... {verdict}", result.name));
    }
    let failed: Vec<_> = results.iter().filter(|result| !result.passed()).collect();
    for result in &failed {
        say(format!(
            "\n---- {} ({}) ----",
            result.name,
            result.path.display()
        ));
        for line in result.failures() {
            say(line);
        }
    }
    let skipped = results.iter().filter(|result| result.skipped()).count();
    let fixme = results
        .iter()
        .filter(|result| matches!(result.disabled(), Some(Disabled::Fixme(_))))
        .count();
    let ignored = results.iter().filter(|result| result.ignored()).count() - fixme;
    say(format!(
        "\ntest result: {}. {} passed; {} failed; {} ignored; {} fixme; {} skipped\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        results.len() - failed.len() - skipped - ignored - fixme,
        failed.len(),
        ignored,
        fixme,
        skipped
    ));
    failed.is_empty()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "tests may panic"
    )]

    use bevy::prelude::*;

    use super::*;
    use crate::{scenario::Step, targets::Target};

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bevy_stagecraft_files_{}_{name}",
            std::process::id()
        ));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("a directory");
        dir
    }

    fn write_scenario(dir: &Path, name: &str, scenario: &Scenario) {
        fs::write(
            dir.join(format!("{name}.ron")),
            scenario.to_ron().expect("exports"),
        )
        .expect("writes");
    }

    fn print_for(dir: &Path, filter: &str, variants: &[Variant]) -> (bool, String, String) {
        print_tagged(dir, filter, &TagFilter::default(), variants)
    }

    fn print_tagged(
        dir: &Path,
        filter: &str,
        tags: &TagFilter,
        variants: &[Variant],
    ) -> (bool, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let passed = run_and_print(
            dir,
            &Registry::default(),
            filter,
            tags,
            variants,
            Scenario::run_headless,
            &mut out,
            &mut err,
        );
        (
            passed,
            String::from_utf8(out).expect("text"),
            String::from_utf8(err).expect("text"),
        )
    }

    /// One step aims at a screen position, which only the default variant may run.
    fn write_pair(dir: &Path) {
        write_scenario(dir, "fine", &Scenario::new("fine").step(Step::wait(1)));
        write_scenario(
            dir,
            "fixed",
            &Scenario::new("fixed")
                .step(Step::move_to(Target::screen(Vec2::new(10.0, 10.0))))
                .step(Step::wait(1)),
        );
    }

    #[test]
    fn a_file_the_filter_excludes_is_not_even_read() {
        let dir = scratch_dir("filter_first");
        write_scenario(&dir, "good", &Scenario::new("good").step(Step::wait(1)));
        fs::write(dir.join("broken.ron"), "this is not a scenario").expect("writes");
        let results = run_ron_variants(
            &dir,
            &Registry::default(),
            "good",
            &[Variant::default()],
            Scenario::run_headless,
        )
        .expect("reads the directory");
        let names: Vec<&str> = results.iter().map(|result| result.name.as_str()).collect();
        assert_eq!(names, ["good"]);
        assert!(results.iter().all(FileResult::passed));
        let everything = run_ron_variants(
            &dir,
            &Registry::default(),
            "",
            &[Variant::default()],
            Scenario::run_headless,
        )
        .expect("reads the directory");
        assert_eq!(everything.len(), 2, "the broken file is a failed result");
    }

    #[test]
    fn a_screen_position_in_a_non_default_variant_is_a_failed_file_result() {
        let dir = scratch_dir("refused_pair");
        write_pair(&dir);
        let variants = [
            Variant::default(),
            Variant::new("narrow").window_size(640, 480),
        ];
        let results = run_ron_variants(
            &dir,
            &Registry::default(),
            "",
            &variants,
            Scenario::run_headless,
        )
        .expect("reads the directory");
        let outcome: Vec<(&str, bool)> = results
            .iter()
            .map(|result| (result.name.as_str(), result.passed()))
            .collect();
        assert_eq!(
            outcome,
            [
                ("fine", true),
                ("fine [narrow]", true),
                ("fixed", true),
                ("fixed [narrow]", false),
            ]
        );
        let refused = results.last().expect("a result");
        assert!(matches!(
            refused.outcome,
            FileOutcome::Ran(Err(LoadError::Invalid { .. }))
        ));
        assert!(
            refused
                .failures()
                .iter()
                .any(|line| line.contains("Screen")),
            "{:?}",
            refused.failures()
        );
    }

    #[test]
    fn the_runner_prints_one_line_per_file_and_variant_and_fails_when_one_failed() {
        let dir = scratch_dir("printed");
        write_pair(&dir);
        let variants = [
            Variant::default(),
            Variant::new("narrow").window_size(640, 480),
        ];
        let (passed, out, err) = print_for(&dir, "", &variants);
        assert!(!passed);
        assert!(err.is_empty(), "{err}");
        for line in [
            "running 4 tests",
            "test fine ... ok",
            "test fine [narrow] ... ok",
            "test fixed ... ok",
            "test fixed [narrow] ... FAILED",
            "---- fixed [narrow] (",
            "test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 fixme; 0 skipped",
        ] {
            assert!(out.contains(line), "missing {line:?} in\n{out}");
        }
    }

    #[test]
    fn the_runner_succeeds_when_every_test_passed_and_the_filter_selects_by_name() {
        let dir = scratch_dir("passing");
        write_pair(&dir);
        let variants = [
            Variant::default(),
            Variant::new("narrow").window_size(640, 480),
        ];
        let (passed, out, _) = print_for(&dir, "fine", &variants);
        assert!(passed, "{out}");
        assert!(out.contains("running 2 tests"), "{out}");
        assert!(
            out.contains("test result: ok. 2 passed; 0 failed; 0 ignored; 0 fixme; 0 skipped"),
            "{out}"
        );
        assert!(!out.contains("fixed"), "{out}");
    }

    #[test]
    fn a_file_restricted_to_other_variants_is_skipped_not_failed() {
        let dir = scratch_dir("skipped");
        write_scenario(
            &dir,
            "only_default",
            &Scenario::new("only_default")
                .variants(["default"])
                .step(Step::move_to(Target::screen(Vec2::new(10.0, 10.0)))),
        );
        let variants = [
            Variant::default(),
            Variant::new("narrow").window_size(640, 480),
            Variant::new("retina").scale_factor(2.0),
        ];
        let results = run_ron_variants(
            &dir,
            &Registry::default(),
            "",
            &variants,
            Scenario::run_headless,
        )
        .expect("reads the directory");
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].variant, "");
        assert!(results[0].passed(), "the default variant runs it");
        assert!(!results[0].skipped());
        assert_eq!(results[1].variant, "narrow");
        assert!(results[1].skipped());
        assert!(results[1].passed(), "a skip is not a failure");
        assert_eq!(results[1].failures().len(), 0);
        assert_eq!(results[2].variant, "retina");
        assert!(
            results[2].skipped(),
            "a file limited to the default variant skips a scale-only variant too"
        );
    }

    #[test]
    fn the_runner_prints_skipped_tests_apart_from_failed_ones() {
        let dir = scratch_dir("printed_skip");
        write_scenario(
            &dir,
            "only_default",
            &Scenario::new("only_default")
                .variants(["default"])
                .step(Step::wait(1)),
        );
        let variants = [
            Variant::default(),
            Variant::new("narrow").window_size(640, 480),
        ];
        let (passed, out, err) = print_for(&dir, "", &variants);
        assert!(passed, "{out}");
        assert!(err.is_empty(), "{err}");
        assert!(out.contains("test only_default ... ok"), "{out}");
        assert!(
            out.contains("test only_default [narrow] ... skipped"),
            "{out}"
        );
        assert!(
            out.contains("test result: ok. 1 passed; 0 failed; 0 ignored; 0 fixme; 1 skipped"),
            "{out}"
        );
    }

    fn failing(name: &str) -> Scenario {
        Scenario::new(name)
            .step(Step::wait(1))
            .expect_check(crate::check::Check::shows("never shown"))
    }

    #[test]
    fn skip_and_fixme_scenarios_are_listed_as_ignored_with_their_reason() {
        let dir = scratch_dir("ignored");
        write_scenario(&dir, "a_skipped", &failing("a_skipped").skip("flaky"));
        write_scenario(&dir, "b_fixme", &failing("b_fixme").fixme("not built"));
        write_scenario(&dir, "c_fine", &Scenario::new("c_fine").step(Step::wait(1)));
        let (passed, out, err) = print_for(&dir, "", &[Variant::default()]);
        assert!(passed, "{out}{err}");
        assert!(
            out.contains("test a_skipped ... ignored, skip: flaky"),
            "{out}"
        );
        assert!(
            out.contains("test b_fixme ... ignored, fixme: not built"),
            "{out}"
        );
        assert!(
            out.contains("1 passed; 0 failed; 1 ignored; 1 fixme; 0 skipped"),
            "{out}"
        );
    }

    #[test]
    fn a_should_fail_scenario_passes_only_by_failing() {
        let dir = scratch_dir("should_fail");
        write_scenario(&dir, "bug", &failing("bug").should_fail("issue 7"));
        let (passed, out, _) = print_for(&dir, "", &[Variant::default()]);
        assert!(passed, "a known bug that still fails is a pass: {out}");
        assert!(
            out.contains("test bug ... ok (failed as expected)"),
            "{out}"
        );
    }

    #[test]
    fn a_should_fail_scenario_that_passes_fails_naming_the_fix() {
        let dir = scratch_dir("fixed_bug");
        write_scenario(
            &dir,
            "bug",
            &Scenario::new("bug")
                .step(Step::wait(1))
                .should_fail("issue 7"),
        );
        let (passed, out, _) = print_for(&dir, "", &[Variant::default()]);
        assert!(!passed, "{out}");
        assert!(
            out.contains("expected to fail (issue 7) but passed"),
            "{out}"
        );
    }

    #[test]
    fn the_tag_filter_keeps_required_tags_and_drops_forbidden_ones() {
        let dir = scratch_dir("tags");
        let wait = |name: &str| Scenario::new(name).step(Step::wait(1));
        write_scenario(&dir, "bridge_fast", &wait("bridge_fast").tags(["bridge"]));
        write_scenario(
            &dir,
            "bridge_slow",
            &wait("bridge_slow").tags(["bridge", "slow"]),
        );
        write_scenario(&dir, "other", &wait("other"));
        let (_, out, _) = print_tagged(
            &dir,
            "",
            &TagFilter::parse("bridge,-slow"),
            &[Variant::default()],
        );
        assert!(out.contains("test bridge_fast ... ok"), "{out}");
        assert!(!out.contains("bridge_slow"), "{out}");
        assert!(!out.contains("other"), "{out}");
    }

    #[test]
    fn tags_and_annotations_round_trip_through_ron() {
        let scenario = Scenario::new("s")
            .tags(["bridge", "slow"])
            .skip("a")
            .fixme("b")
            .should_fail("c")
            .step(Step::wait(1));
        let text = scenario.to_ron().expect("exports");
        let back = Scenario::from_ron(&text).expect("loads");
        assert_eq!(back.tag_names(), ["bridge", "slow"]);
        assert_eq!(back.skip_reason(), Some("a"));
        assert_eq!(back.fixme_reason(), Some("b"));
        assert_eq!(back.should_fail_reason(), Some("c"));
        assert_eq!(back.to_script(), scenario.to_script());
    }

    #[test]
    fn a_file_may_write_tags_and_annotations() {
        let scenario = Scenario::from_ron(
            "(format_version: 3, name: \"s\", tags: [\"x\"], skip: Some(\"why\"), steps: [])",
        )
        .expect("loads");
        assert_eq!(scenario.tag_names(), ["x"]);
        assert_eq!(scenario.skip_reason(), Some("why"));
    }

    #[test]
    fn the_runner_fails_naming_a_directory_it_cannot_read() {
        let dir = scratch_dir("missing").join("nothing_here");
        let (passed, out, err) = print_for(&dir, "", &[Variant::default()]);
        assert!(!passed);
        assert!(out.is_empty(), "{out}");
        assert!(err.contains("nothing_here"), "{err}");
    }
}
