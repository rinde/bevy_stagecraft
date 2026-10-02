//! Turning a directory of `.ron` scenarios into tests.
//!
//! Add a test target with `harness = false` and call [`run_ron_tests`] from
//! its `main`; every `.ron` file in the directory becomes one test, named
//! after the file.
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
    path::{Path, PathBuf},
    process::ExitCode,
};

use crate::{
    player::ScenarioReport,
    scenario::Scenario,
    script::{LoadError, Registry},
};

/// One `.ron` file played.
#[derive(Debug, Clone, PartialEq)]
pub struct FileResult {
    /// The file name without its extension, which is the test's name.
    pub name: String,
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

/// The `.ron` files directly in `dir`, sorted by name.
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
        if path.extension().is_some_and(|extension| extension == "ron") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Loads one file as a scenario.
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
    Scenario::from_ron_with(&text, registry)
}

/// Loads and plays every `.ron` file in `dir` whose name contains `filter`
/// (all of them when it is empty), with `run`.
///
/// # Errors
///
/// [`LoadError::Io`] if the directory cannot be read. A file that does not
/// load is a failed [`FileResult`], not an error here.
pub fn run_ron_files(
    dir: impl AsRef<Path>,
    registry: &Registry,
    filter: &str,
    mut run: impl FnMut(&Scenario) -> ScenarioReport,
) -> Result<Vec<FileResult>, LoadError> {
    Ok(ron_files(dir)?
        .into_iter()
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().into_owned();
            name.contains(filter).then_some((name, path))
        })
        .map(|(name, path)| {
            let outcome = load_ron_file(&path, registry).map(|scenario| run(&scenario));
            FileResult {
                name,
                path,
                outcome,
            }
        })
        .collect())
}

/// The `main` of a `harness = false` test target.
///
/// Plays every `.ron` file in `dir`, prints one line per file in the style of
/// the standard harness, and succeeds only if all of them passed. Command-line
/// words that do not start with `-` filter by file name, as with
/// `cargo test <filter>`.
#[must_use]
#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this is a test harness: its output is the test report"
)]
pub fn run_ron_tests(
    dir: impl AsRef<Path>,
    registry: &Registry,
    run: impl FnMut(&Scenario) -> ScenarioReport,
) -> ExitCode {
    let filter = std::env::args()
        .skip(1)
        .find(|argument| !argument.starts_with('-'))
        .unwrap_or_default();
    let results = match run_ron_files(dir, registry, &filter, run) {
        Ok(results) => results,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    println!("\nrunning {} tests", results.len());
    for result in &results {
        println!(
            "test {} ... {}",
            result.name,
            if result.passed() { "ok" } else { "FAILED" }
        );
    }
    let failed: Vec<_> = results.iter().filter(|result| !result.passed()).collect();
    for result in &failed {
        println!("\n---- {} ({}) ----", result.name, result.path.display());
        for line in result.failures() {
            println!("{line}");
        }
    }
    println!(
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
