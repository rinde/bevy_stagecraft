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
        let loaded = load_ron_file(&path, registry);
        for variant in variants {
            let name = variant.label(&stem);
            if !name.contains(filter) {
                continue;
            }
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

/// Write a line to standard output; a closed pipe is not worth failing over.
macro_rules! say {
    ($($arg:tt)*) => {
        writeln!(std::io::stdout().lock(), $($arg)*).ok()
    };
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
    let results = match run_ron_variants(dir, registry, &filter, variants, run) {
        Ok(results) => results,
        Err(error) => {
            writeln!(std::io::stderr().lock(), "{error}").ok();
            return ExitCode::FAILURE;
        }
    };
    say!("\nrunning {} tests", results.len());
    for result in &results {
        say!(
            "test {} ... {}",
            result.name,
            if result.passed() { "ok" } else { "FAILED" }
        );
    }
    let failed: Vec<_> = results.iter().filter(|result| !result.passed()).collect();
    for result in &failed {
        say!("\n---- {} ({}) ----", result.name, result.path.display());
        for line in result.failures() {
            say!("{line}");
        }
    }
    say!(
        "\ntest result: {}. {} passed; {} failed\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        results.len() - failed.len(),
        failed.len()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
