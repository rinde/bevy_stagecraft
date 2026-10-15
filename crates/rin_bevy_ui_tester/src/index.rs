//! What the recorder writes for each scenario, and the page that lists them.

use core::fmt::Write as _;
use std::{fs, path::Path};

use crate::{
    capture::{FilmstripCell, Frame, Mark, encode_gif, filmstrip},
    check::{ScreenshotRecord, ScreenshotResult, suggest::SnapshotChange},
    overlay::TargetDescriptions,
    player::{CheckKind, Outcome, StepRecord},
    recorder::{Capture, RecordError, ScenarioRecording, create_dir, write_file},
    trace::{ScenarioTrace, StepView, TracePoint, step_title},
};

/// Width of a thumbnail in a filmstrip, in pixels.
pub const FILMSTRIP_THUMB_WIDTH: u32 = 320;
/// How many thumbnails a filmstrip puts in a row.
pub const FILMSTRIP_COLUMNS: usize = 4;
/// Width of the frames of a GIF, in pixels.
pub const GIF_WIDTH: u32 = 640;
/// Milliseconds in one 1/60 s frame, times 1000 over 60; see [`delay_ms`].
const FRAME_MS_NUMERATOR: u32 = 1000;
/// Frames a second.
const FRAMES_PER_SECOND: u32 = 60;
/// How long the last frame of a GIF stays up, in milliseconds.
const LAST_FRAME_MS: u32 = 1500;
/// The shortest a GIF frame is shown; browsers treat shorter as 100 ms.
const MIN_FRAME_MS: u32 = 20;
/// File name of a scenario's filmstrip.
pub const FILMSTRIP_FILE: &str = "filmstrip.png";
/// File name of a scenario's GIF.
pub const GIF_FILE: &str = "scenario.gif";
/// File name of a scenario's per-step trace.
pub const TRACE_FILE: &str = "trace.html";
/// The folder of a scenario's folder that holds copies of the pictures its
/// screenshot checks wrote, so the page links to files that sit beside it.
pub const SNAPSHOT_DIR: &str = "snapshots";

/// How long a GIF shows a picture taken at `frame` before the one at `next`.
fn delay_ms(frame: u32, next: Option<u32>) -> u32 {
    next.map_or(LAST_FRAME_MS, |next| {
        (next.saturating_sub(frame) * FRAME_MS_NUMERATOR / FRAMES_PER_SECOND).max(MIN_FRAME_MS)
    })
}

/// The file name of the picture of `frame`.
#[must_use]
pub fn picture_file(frame: u32) -> String {
    format!("frame-{frame:05}.png")
}

/// Writes a scenario's pictures, filmstrip, GIF and, where it has a trace,
/// `trace.html` into `folder`.
pub(crate) fn write_scenario(
    folder: &Path,
    pictures: &[Capture],
    recording: &ScenarioRecording,
) -> Result<(), RecordError> {
    create_dir(folder)?;
    for picture in pictures {
        write_file(
            &folder.join(picture_file(picture.meta.frame)),
            &picture.frame.to_png()?,
        )?;
    }
    let cells: Vec<FilmstripCell> = pictures
        .iter()
        .map(|picture| FilmstripCell {
            frame: picture.frame.scaled_to_width(FILMSTRIP_THUMB_WIDTH),
            mark: if picture.meta.frame == 0 {
                Mark::Neutral
            } else if picture.meta.failed {
                Mark::Fail
            } else {
                Mark::Pass
            },
            number: picture.meta.frame,
        })
        .collect();
    if let Some(strip) = filmstrip(&cells, FILMSTRIP_COLUMNS) {
        write_file(&folder.join(FILMSTRIP_FILE), &strip.to_png()?)?;
    }
    let gif_frames: Vec<(Frame, u32)> = pictures
        .iter()
        .enumerate()
        .map(|(index, picture)| {
            let next = pictures
                .get(index + 1)
                .map(|following| following.meta.frame);
            (
                picture.frame.scaled_to_width(GIF_WIDTH),
                delay_ms(picture.meta.frame, next),
            )
        })
        .collect();
    if !gif_frames.is_empty() {
        write_file(&folder.join(GIF_FILE), &encode_gif(&gif_frames)?)?;
    }
    copy_snapshots(folder, &recording.report.screenshots)?;
    if let Some(trace) = &recording.trace {
        let frames: Vec<u32> = pictures.iter().map(|picture| picture.meta.frame).collect();
        write_file(
            &folder.join(TRACE_FILE),
            trace_html(recording, trace, &frames).as_bytes(),
        )?;
    }
    Ok(())
}

/// The files of a screenshot check worth looking at: its baseline and, after a
/// mismatch, the picture taken and the one of what differs. Each is named
/// `<check>.<role>.png` in the scenario's [`SNAPSHOT_DIR`].
fn snapshot_files(record: &ScreenshotRecord) -> Vec<(String, &Path)> {
    let mut files = Vec::new();
    match &record.result {
        ScreenshotResult::Matched | ScreenshotResult::Updated => return files,
        ScreenshotResult::Created => {}
        ScreenshotResult::Mismatch { actual, diff } => {
            files.push((format!("{}.actual.png", record.name), actual.as_path()));
            if let Some(diff) = diff {
                files.push((format!("{}.diff.png", record.name), diff.as_path()));
            }
        }
    }
    files.insert(
        0,
        (
            format!("{}.baseline.png", record.name),
            record.baseline.as_path(),
        ),
    );
    files
}

/// Copies the pictures screenshot checks wrote into the scenario's folder.
/// A baseline that is not there (a mismatch in size has one, a failed write
/// does not) is left out.
fn copy_snapshots(folder: &Path, records: &[ScreenshotRecord]) -> Result<(), RecordError> {
    let files: Vec<_> = records.iter().flat_map(snapshot_files).collect();
    if files.is_empty() {
        return Ok(());
    }
    let directory = folder.join(SNAPSHOT_DIR);
    create_dir(&directory)?;
    for (name, from) in files {
        if from.exists() {
            let to = directory.join(name);
            fs::copy(from, &to).map_err(|error| RecordError::Io(to, error))?;
        }
    }
    Ok(())
}

/// Links, on a scenario's card, to the pictures screenshot checks left to look
/// at.
fn snapshot_links(out: &mut String, slug: &str, records: &[ScreenshotRecord]) {
    for record in records {
        let links: Vec<String> = snapshot_files(record)
            .into_iter()
            .map(|(file, _)| {
                let label = file.rsplit('.').nth(1).unwrap_or("picture").to_owned();
                format!(
                    "<a href=\"{slug}/{SNAPSHOT_DIR}/{href}\">{label}</a>",
                    href = escape(&file),
                )
            })
            .collect();
        if !links.is_empty() {
            let _ = write!(
                out,
                "<li class=\"fail\">screenshot `{}`: {}</li>",
                escape(&record.name),
                links.join(" \u{b7} ")
            );
        }
    }
}

/// Escapes text for HTML.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// One scenario's card in the contact sheet.
fn card(out: &mut String, scenario: &ScenarioRecording) {
    let (class, verdict) = if scenario.skipped() {
        ("skipped", "SKIPPED")
    } else if scenario.passed() {
        ("pass", "passed")
    } else {
        ("fail", "FAILED")
    };
    let slug = escape(&scenario.slug);
    let _ = write!(
        out,
        "<article class=\"card {class}\" id=\"{slug}\"><h3>{name} <small>{verdict}</small></h3>",
        name = escape(&scenario.name),
    );
    if scenario.skipped() {
        out.push_str("<p>does not run in this variant</p></article>");
        return;
    }
    let _ = write!(
        out,
        "<a href=\"{slug}/{FILMSTRIP_FILE}\"><img src=\"{slug}/{GIF_FILE}\" alt=\"{name}\" \
         loading=\"lazy\"></a>",
        name = escape(&scenario.name),
    );
    out.push_str("<ul>");
    for check in &scenario.report.checks {
        let (mark, class) = match check.outcome {
            Outcome::Pass => ("\u{2714}", "pass"),
            Outcome::Fail(_) => ("\u{2718}", "fail"),
            Outcome::Skipped => ("-", "skipped"),
        };
        let when = match check.kind {
            CheckKind::Step => "",
            CheckKind::Expect => " (at the end)",
        };
        let _ = write!(
            out,
            "<li class=\"{class}\">{mark} {}{when}</li>",
            escape(&check.name)
        );
    }
    for failure in scenario.report.failures() {
        let _ = write!(out, "<li class=\"fail\">{}</li>", escape(&failure));
    }
    snapshot_links(out, &slug, &scenario.report.screenshots);
    out.push_str("</ul>");
    if !scenario.passed() && scenario.trace.is_some() {
        card_steps(out, scenario, &slug);
    }
    let trace_link = if scenario.trace.is_some() {
        format!(" \u{b7} <a href=\"{slug}/{TRACE_FILE}\">trace</a>")
    } else {
        String::new()
    };
    let _ = write!(
        out,
        "<p><a href=\"{slug}/{FILMSTRIP_FILE}\">filmstrip</a>{trace_link} \u{b7} \
         <a href=\"{slug}/\">{} pictures</a></p></article>",
        scenario.pictures
    );
}

/// What a step is called in a list: its number and its words.
fn title(index: usize, steps: &[StepRecord]) -> String {
    step_title(index, steps, &TargetDescriptions::default()).unwrap_or_default()
}

/// The mark and class a step's outcome is shown with.
const fn outcome_mark(outcome: &Outcome) -> (&'static str, &'static str) {
    match outcome {
        Outcome::Pass => ("\u{2714}", "pass"),
        Outcome::Fail(_) => ("\u{2718}", "fail"),
        Outcome::Skipped => ("-", "skipped"),
    }
}

/// The steps of a failed scenario, each linking to its part of the trace.
fn card_steps(out: &mut String, scenario: &ScenarioRecording, slug: &str) {
    out.push_str("<ol class=\"steps\">");
    for (index, record) in scenario.report.steps.iter().enumerate() {
        let (mark, class) = outcome_mark(&record.outcome);
        let _ = write!(
            out,
            "<li class=\"{class}\"><a href=\"{slug}/{TRACE_FILE}#step-{index}\">{mark} {}</a></li>",
            escape(&title(index, &scenario.report.steps))
        );
    }
    out.push_str("</ol>");
}

/// The page that shows, per step, the world around it.
///
/// Each step has what it found, the checks it ran, the pictures before and
/// after, how the snapshot changed, and the snapshot and UI tree afterwards.
/// `frames` are the frames that have a picture in the folder next to the page.
#[must_use]
pub fn trace_html(recording: &ScenarioRecording, trace: &ScenarioTrace, frames: &[u32]) -> String {
    let mut out = String::from(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Trace</title><style>\
         body{font-family:sans-serif;background:#18181c;color:#eee;margin:1.5rem}\
         details{background:#24242c;border-left:6px solid #3c9a54;margin:.5rem 0;padding:.4rem .8rem}\
         details.fail{border-left-color:#c84040}details.skipped{border-left-color:#777}\
         summary{cursor:pointer;font-weight:bold}\
         .pictures{display:flex;gap:1rem;flex-wrap:wrap}\
         .pictures figure{margin:.4rem 0;max-width:46%}.pictures img{width:100%}\
         table{border-collapse:collapse;font-size:.85rem}\
         td,th{border:1px solid #444;padding:.1rem .5rem;text-align:left;vertical-align:top}\
         td.added{color:#7fdc8c}td.removed{color:#ff7a7a}\
         pre{background:#101014;padding:.4rem;overflow:auto;font-size:.8rem}\
         li.pass{color:#7fdc8c}li.fail{color:#ff7a7a}li.skipped{color:#999}\
         a{color:#9cc8ff}</style></head><body>",
    );
    let _ = write!(
        out,
        "<h1>{}</h1><p><a href=\"../index.html#{}\">back to the index</a></p>",
        escape(&recording.name),
        escape(&recording.slug)
    );
    if let Some(failure) = &recording.report.setup_failure {
        let _ = write!(
            out,
            "<p class=\"fail\">The setup failed: {}</p>",
            escape(failure)
        );
    }
    let steps = &recording.report.steps;
    let descriptions = TargetDescriptions::default();
    for index in 0..steps.len() {
        if let Some(view) = StepView::new(index, steps, trace, &descriptions, trace.filter()) {
            step_section(&mut out, &view, frames);
        }
    }
    end_section(&mut out, recording, trace);
    out.push_str("</body></html>\n");
    out
}

/// One step's `<details>`; a step that failed is open.
fn step_section(out: &mut String, view: &StepView<'_>, frames: &[u32]) {
    let (mark, class) = outcome_mark(&view.outcome);
    let open = if view.failure().is_some() {
        " open"
    } else {
        ""
    };
    let _ = write!(
        out,
        "<details id=\"step-{}\" class=\"{class}\"{open}><summary>{mark} {}</summary>",
        view.index,
        escape(&view.title)
    );
    if let Some(message) = view.failure() {
        let _ = write!(out, "<p class=\"fail\">{}</p>", escape(message));
    }
    if !view.ran() {
        out.push_str("<p>Never ran, because an earlier step failed.</p></details>");
        return;
    }
    if !view.targets.is_empty() {
        out.push_str("<h4>Targets</h4><ul>");
        for target in view.targets {
            let class = if target.is_found() { "pass" } else { "fail" };
            let _ = write!(
                out,
                "<li class=\"{class}\">{} \u{2192} {}</li>",
                escape(&target.target),
                escape(target.answer())
            );
        }
        out.push_str("</ul>");
    }
    if !view.captured.is_empty() {
        out.push_str("<h4>Captured</h4><ul>");
        for captured in view.captured {
            let _ = write!(out, "<li>{}</li>", escape(&captured.describe()));
        }
        out.push_str("</ul>");
    }
    if !view.checks.is_empty() {
        out.push_str("<h4>Checks</h4><ul>");
        for check in view.checks {
            let (mark, class) = outcome_mark(&check.outcome);
            let why = match check.failure() {
                "" => String::new(),
                message => format!(": {}", escape(message)),
            };
            let _ = write!(
                out,
                "<li class=\"{class}\">{mark} {}{why}</li>",
                escape(&check.name)
            );
        }
        out.push_str("</ul>");
    }
    pictures(out, view.before, view.after, frames);
    if view.before.is_some() && view.after.is_some() {
        changes(out, &view.changes);
    }
    if let Some(after) = view.after {
        point(out, "After the step", after);
    }
    out.push_str("</details>");
}

/// The pictures before and after a step, where the recorder took them.
fn pictures(
    out: &mut String,
    before: Option<&TracePoint>,
    after: Option<&TracePoint>,
    frames: &[u32],
) {
    let shown: Vec<(&str, u32)> = [("Before", before), ("After", after)]
        .into_iter()
        .filter_map(|(label, point)| Some((label, point?.frame)))
        .filter(|(_, frame)| frames.contains(frame))
        .collect();
    if shown.is_empty() {
        return;
    }
    out.push_str("<div class=\"pictures\">");
    for (label, frame) in shown {
        let _ = write!(
            out,
            "<figure><figcaption>{label} (frame {frame})</figcaption>\
             <img src=\"{}\" alt=\"{label}\" loading=\"lazy\"></figure>",
            picture_file(frame)
        );
    }
    out.push_str("</div>");
}

/// The changes between two snapshots as a table.
fn changes(out: &mut String, changes: &[SnapshotChange]) {
    if changes.is_empty() {
        out.push_str("<h4>Changes</h4><p>The world did not change.</p>");
        return;
    }
    out.push_str("<h4>Changes</h4><table><tr><th>What</th><th>Before</th><th>After</th></tr>");
    for change in changes {
        let cell = |value: &Option<String>, class: &str| {
            value.as_ref().map_or_else(
                || format!("<td class=\"{class}\">\u{2014}</td>"),
                |text| format!("<td>{}</td>", escape(text)),
            )
        };
        let _ = write!(
            out,
            "<tr><td>{}</td>{}{}</tr>",
            escape(&change.key),
            cell(&change.before, "removed"),
            cell(&change.after, "added")
        );
    }
    out.push_str("</table>");
}

/// A trace point's UI tree and snapshot, collapsed.
fn point(out: &mut String, title: &str, point: &TracePoint) {
    let mut tree = String::new();
    for line in &point.ui {
        let _ = writeln!(
            tree,
            "{}{}",
            "  ".repeat(line.depth),
            escape(&line.describe())
        );
    }
    let mut snapshot = String::new();
    for (key, value) in point.snapshot.entries() {
        let _ = writeln!(snapshot, "{} = {}", escape(&key), escape(&value));
    }
    let _ = write!(
        out,
        "<details><summary>{title} (frame {}): UI and snapshot</summary>\
         <h4>UI</h4><pre>{tree}</pre><h4>Snapshot</h4><pre>{snapshot}</pre></details>",
        point.frame
    );
}

/// The end of the run: the expectations and the final world.
fn end_section(out: &mut String, recording: &ScenarioRecording, trace: &ScenarioTrace) {
    let failed =
        recording.report.checks.iter().any(|check| {
            check.kind == CheckKind::Expect && matches!(check.outcome, Outcome::Fail(_))
        });
    let class = if failed { "fail" } else { "pass" };
    let _ = write!(
        out,
        "<details id=\"end\" class=\"{class}\"{}><summary>End of the run</summary><ul>",
        if failed { " open" } else { "" }
    );
    for check in recording
        .report
        .checks
        .iter()
        .filter(|check| check.kind == CheckKind::Expect)
    {
        let (mark, class) = outcome_mark(&check.outcome);
        let why = match &check.outcome {
            Outcome::Fail(message) => format!(": {}", escape(message)),
            _ => String::new(),
        };
        let _ = write!(
            out,
            "<li class=\"{class}\">{mark} {}{why}</li>",
            escape(&check.name)
        );
    }
    out.push_str("</ul>");
    if let Some(last) = trace.last() {
        point(out, "The world at the end", last);
    }
    out.push_str("</details>");
}

/// The contact sheet: scenarios grouped as the catalogue groups them, groups
/// with a failure first, and failures first within each group.
#[must_use]
pub fn index_html(scenarios: &[ScenarioRecording]) -> String {
    let mut groups: Vec<(&str, Vec<&ScenarioRecording>)> = Vec::new();
    for scenario in scenarios {
        match groups
            .iter_mut()
            .find(|(group, _)| *group == scenario.group)
        {
            Some((_, members)) => members.push(scenario),
            None => groups.push((&scenario.group, vec![scenario])),
        }
    }
    // Stable sorts keep the catalogue's order among equals.
    for (_, members) in &mut groups {
        members.sort_by_key(|scenario| !scenario.failed());
    }
    groups.sort_by_key(|(_, members)| !members.iter().any(|scenario| scenario.failed()));

    let failed = scenarios
        .iter()
        .filter(|scenario| scenario.failed())
        .count();
    let mut out = String::from(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Scenario recordings</title><style>\
         body{font-family:sans-serif;background:#18181c;color:#eee;margin:1.5rem}\
         .grid{display:flex;flex-wrap:wrap;gap:1rem}\
         .card{width:340px;background:#24242c;border-left:6px solid #3c9a54;padding:.5rem .8rem}\
         .card.fail{border-left-color:#c84040}\
         .card.skipped{border-left-color:#777}\
         .card img{width:100%}\
         .card h3{margin:.2rem 0}.card small{font-weight:normal;opacity:.7}\
         ul{list-style:none;padding:0;margin:.4rem 0;font-size:.9rem}\
         li.pass{color:#7fdc8c}li.fail{color:#ff7a7a}li.skipped{color:#999}\
         a{color:#9cc8ff}</style></head><body><h1>Scenario recordings</h1>",
    );
    let skipped = scenarios
        .iter()
        .filter(|scenario| scenario.skipped())
        .count();
    let _ = write!(
        out,
        "<p>{} scenarios, {failed} failed{}.</p>",
        scenarios.len(),
        if skipped == 0 {
            String::new()
        } else {
            format!(", {skipped} skipped")
        }
    );
    for (group, members) in groups {
        let _ = write!(
            out,
            "<section><h2>{}</h2><div class=\"grid\">",
            escape(group)
        );
        for scenario in members {
            card(&mut out, scenario);
        }
        out.push_str("</div></section>");
    }
    out.push_str("</body></html>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Step, player::ScenarioReport};
    use bevy::prelude::{Reflect, ReflectResource, Resource};

    /// A recording with a finished report that passed or not.
    fn recording(name: &str, group: &str, passed: bool) -> ScenarioRecording {
        let mut report = ScenarioReport {
            finished: true,
            ..ScenarioReport::default()
        };
        if !passed {
            report.checks.push(crate::player::CheckRecord {
                name: "broken <check>".into(),
                kind: CheckKind::Expect,
                outcome: Outcome::Fail("no".into()),
            });
        }
        ScenarioRecording {
            name: name.into(),
            group: group.into(),
            slug: name.into(),
            report,
            skipped: false,
            pictures: 3,
            trace: None,
        }
    }

    /// A recording of a scenario that does not run in its variant.
    fn skipped_recording(name: &str, group: &str) -> ScenarioRecording {
        ScenarioRecording {
            name: name.into(),
            group: group.into(),
            slug: name.into(),
            report: ScenarioReport::default(),
            skipped: true,
            pictures: 0,
            trace: None,
        }
    }

    #[test]
    fn the_index_shows_a_skipped_scenario_as_skipped_not_failed() {
        let html = index_html(&[
            skipped_recording("elsewhere", "A"),
            recording("good", "A", true),
        ]);
        assert!(html.contains("SKIPPED"), "it is marked as skipped: {html}");
        assert!(html.contains("1 skipped"), "the summary counts it: {html}");
        assert!(
            !html.contains("1 failed"),
            "a skip is not a failure: {html}"
        );
        assert!(
            !html.contains("elsewhere/filmstrip.png"),
            "nothing was recorded for it: {html}"
        );
    }

    #[test]
    fn the_index_lists_failures_first_and_escapes_text() {
        let html = index_html(&[
            recording("good", "A", true),
            recording("bad", "B", false),
            recording("fine", "B", true),
        ]);
        let bad = html.find("id=\"bad\"").unwrap_or(usize::MAX);
        let good = html.find("id=\"good\"").unwrap_or(0);
        assert!(bad < good, "group B has a failure, so it comes first");
        assert!(html.contains("broken &lt;check&gt;"), "names are escaped");
        assert!(html.contains("1 failed"), "the summary counts failures");
    }

    /// A recording of a scenario that fails its check, with its trace.
    fn traced_failure() -> ScenarioRecording {
        let scenario = crate::Scenario::new("shows it")
            .setup(|world| {
                world.spawn(bevy::prelude::Camera2d);
                world.spawn((
                    bevy::ui::Node::default(),
                    bevy::ui::widget::Text::new("Hello <b>"),
                ));
            })
            .steps([Step::wait(2), Step::check(crate::Check::shows("Missing"))]);
        let (report, trace) = scenario.run_headless_traced();
        ScenarioRecording {
            name: "shows it".into(),
            group: "A".into(),
            slug: "shows-it".into(),
            report,
            skipped: false,
            pictures: 0,
            trace: Some(trace),
        }
    }

    #[test]
    fn a_trace_page_has_a_section_per_step_and_marks_the_failed_one() {
        let recording = traced_failure();
        let Some(trace) = &recording.trace else {
            return;
        };
        let html = trace_html(&recording, trace, &[]);
        assert!(html.contains("id=\"step-0\""), "{html}");
        assert!(html.contains("id=\"step-1\""), "{html}");
        assert!(html.contains("class=\"fail\""), "the failed step is marked");
        assert!(html.contains("Hello &lt;b&gt;"), "the UI tree is escaped");
        assert!(!html.contains("<img"), "no picture was taken");
    }

    /// A resource a `Capture` step can read.
    #[derive(Resource, Reflect)]
    #[reflect(Resource)]
    struct Tally(u32);

    #[test]
    fn a_trace_page_lists_what_a_step_captured_escaped() {
        let scenario = crate::Scenario::new("captures")
            .setup(|world| {
                world.insert_resource(Tally(7));
            })
            .steps([Step::capture(
                "a <b>",
                crate::Measure::resource::<Tally>("0"),
            )]);
        let mut app = bevy::prelude::App::new();
        app.add_plugins(crate::HeadlessPlugins::default())
            .register_type::<Tally>();
        app.finish();
        app.cleanup();
        let (report, trace) = scenario.run_in_traced(&mut app, crate::NoiseFilter::default());
        assert!(report.passed(), "{:?}", report.failures());
        let recording = ScenarioRecording {
            name: "captures".into(),
            group: "A".into(),
            slug: "captures".into(),
            report,
            skipped: false,
            pictures: 0,
            trace: Some(trace.clone()),
        };
        let html = trace_html(&recording, &trace, &[]);
        assert!(html.contains("<h4>Captured</h4>"), "{html}");
        assert!(
            html.contains("&quot;a &lt;b&gt;&quot; = 7 (u32), `Tally.0`"),
            "{html}"
        );
    }

    #[test]
    fn a_failed_scenario_with_a_trace_links_its_steps_from_the_index() {
        let html = index_html(&[traced_failure()]);
        assert!(html.contains("shows-it/trace.html#step-1"), "{html}");
    }

    #[test]
    fn gif_delays_follow_the_frame_gaps() {
        assert_eq!(delay_ms(0, Some(6)), 100, "six frames at 60 fps");
        assert_eq!(
            delay_ms(0, Some(1)),
            MIN_FRAME_MS,
            "never below the minimum"
        );
        assert_eq!(delay_ms(5, None), LAST_FRAME_MS, "the last picture lingers");
    }
}
