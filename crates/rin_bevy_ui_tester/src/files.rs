//! Turning a directory of `.ron` scenarios into tests.
//!
//! Add a test target with `harness = false` and call [`run_ron_tests`] from
//! its `main`; every `.ron` file in the directory becomes one test, named
//! after the file. With [`run_ron_tests_with_variants`] every file is played
//! once per [`Variant`] (window size, scale factor, theme), as one test named
//! `file [variant]`.
//!
//! ```toml
//! [[test]]
//! name = "ron_scenarios"
//! harness = false
//! ```
//!
//! ```ignore
//! fn main() -> std::process::ExitCode {
//!     rin_bevy_ui_tester::run_ron_tests("tests/scenarios", &registry(), |scenario| {
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
    player::ScenarioReport,
    save::is_scene_file,
    scenario::Scenario,
    script::{LoadError, Registry},
    variant::Variant,
};

/// One `.ron` file played.
#[derive(Debug, Clone, PartialEq)]
pub struct FileResult {
    /// The test's name: the file name without its extension, and the
    /// variant's name in brackets after it unless the variant is the default
    /// one (`wrap [narrow]`, see [`Variant::label`]).
    pub name: String,
    /// The name of the variant the file was played in; empty for the default
    /// one.
    pub variant: String,
    /// The file.
    pub path: PathBuf,
    /// The report, or why the file did not load.
    pub outcome: Result<ScenarioReport, LoadError>,
}

impl FileResult {
    /// Whether the file loaded and its scenario passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.outcome.as_ref().is_ok_and(ScenarioReport::passed)
    }

    /// Why the test failed, one line per reason; empty when it passed.
    #[must_use]
    pub fn failures(&self) -> Vec<String> {
        match &self.outcome {
            Ok(report) if report.passed() => Vec::new(),
            Ok(report) => {
                let mut failures = report.failures();
                if failures.is_empty() {
                    failures.push("the scenario did not finish".into());
                }
                failures
            }
            Err(error) => vec![error.to_string()],
        }
    }
}

/// The `.ron` files directly in `dir`, sorted by name, except the saved
/// worlds (`*.scene.ron`, see [`crate::save`]), which are setups and not
/// scenarios.
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

/// Loads one file as a scenario. A setup in it that names a file (a saved
/// world, `Scene("..")`) looks for it next to this one.
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

/// Loads and plays every `.ron` file in `dir` whose name contains `filter`
/// (all of them when it is empty), with `run`, as written: the default
/// variant only. See [`run_ron_variants`].
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

/// Loads every `.ron` file in `dir` and plays it once per variant in
/// `variants`, with `run`, named `file [variant]`.
///
/// `run` gets the scenario as that variant runs it (see [`Variant::apply`]).
/// A file and variant are played when that name contains `filter` (all of
/// them when it is empty).
///
/// Only the variants listed are played: include [`Variant::default`] to play
/// the file as written too. A variant a file cannot run in (for example a
/// step aimed at a screen position, in a variant with another window size) is
/// a failed [`FileResult`] that says so.
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
        // A file the filter leaves nothing of is not even read: it may be
        // broken, and parsing it costs time for nothing.
        if wanted.is_empty() {
            continue;
        }
        let loaded = load_ron_file(&path, registry);
        for (variant, name) in wanted {
            let outcome = loaded
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|scenario| variant.apply(scenario))
                .map(|scenario| run(&scenario));
            results.push(FileResult {
                name,
                variant: variant.name().to_owned(),
                path: path.clone(),
                outcome,
            });
        }
    }
    Ok(results)
}

/// The `main` of a `harness = false` test target.
///
/// Plays every `.ron` file in `dir`, prints one line per file in the style of
/// the standard harness, and succeeds only if all of them passed. Command-line
/// words that do not start with `-` filter by file name, as with
/// `cargo test <filter>`. See [`run_ron_tests_with_variants`] to play every
/// file in several windows and themes.
#[must_use]
pub fn run_ron_tests(
    dir: impl AsRef<Path>,
    registry: &Registry,
    run: impl FnMut(&Scenario) -> ScenarioReport,
) -> ExitCode {
    run_ron_tests_with_variants(dir, registry, &[Variant::default()], run)
}

/// The `main` of a `harness = false` test target that plays every `.ron` file
/// once per variant, as [`run_ron_variants`] does.
///
/// Prints one line per file and variant, named `file [variant]` (just `file`
/// in the default variant), and succeeds only if all of them passed. A filter
/// word on the command line matches those names, so `cargo test narrow` plays
/// the narrow variant of every file.
#[must_use]
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

/// Plays the files as [`run_ron_variants`] does, prints the standard harness's
/// lines to `out` (a directory that cannot be read goes to `err`) and returns
/// whether every test passed. It is the body of
/// [`run_ron_tests_with_variants`], split out so the lines and the verdict can
/// be tested.
fn run_and_print(
    dir: &Path,
    registry: &Registry,
    filter: &str,
    variants: &[Variant],
    run: impl FnMut(&Scenario) -> ScenarioReport,
    out: &mut impl Write,
    err: &mut impl Write,
) -> bool {
    match run_ron_variants(dir, registry, filter, variants, run) {
        Ok(results) => print_results(out, &results),
        Err(error) => {
            // A closed pipe is not worth failing over.
            writeln!(err, "{error}").ok();
            false
        }
    }
}

/// Writes one line per result in the style of the standard harness, then the
/// failures and the summary. Whether all of them passed.
fn print_results(out: &mut impl Write, results: &[FileResult]) -> bool {
    // A closed pipe is not worth failing over.
    let mut say = |line: String| writeln!(out, "{line}").ok();
    say(format!("\nrunning {} tests", results.len()));
    for result in results {
        say(format!(
            "test {} ... {}",
            result.name,
            if result.passed() { "ok" } else { "FAILED" }
        ));
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
    say(format!(
        "\ntest result: {}. {} passed; {} failed\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        results.len() - failed.len(),
        failed.len()
    ));
    failed.is_empty()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "tests may panic")]

    use bevy::prelude::*;

    use super::*;
    use crate::{scenario::Step, targets::Target};

    /// A fresh directory for one test, under the system's temporary one.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rin_bevy_ui_tester_files_{}_{name}",
            std::process::id()
        ));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("a directory");
        dir
    }

    /// Writes `scenario` to `dir/name.ron`.
    fn write_scenario(dir: &Path, name: &str, scenario: &Scenario) {
        fs::write(
            dir.join(format!("{name}.ron")),
            scenario.to_ron().expect("exports"),
        )
        .expect("writes");
    }

    /// What `run_and_print` printed and decided.
    fn print_for(dir: &Path, filter: &str, variants: &[Variant]) -> (bool, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let passed = run_and_print(
            dir,
            &Registry::default(),
            filter,
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

    /// A file that waits one frame, and one that also aims at a screen
    /// position, which no variant but the default may do.
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
        assert!(matches!(refused.outcome, Err(LoadError::Invalid { .. })));
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
            "test result: FAILED. 3 passed; 1 failed",
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
        assert!(out.contains("test result: ok. 2 passed; 0 failed"), "{out}");
        assert!(!out.contains("fixed"), "{out}");
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
