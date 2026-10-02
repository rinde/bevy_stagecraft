//! What the recorder writes for each scenario, and the page that lists them.

use core::fmt::Write as _;
use std::path::Path;

use crate::{
    capture::{FilmstripCell, Frame, Mark, encode_gif, filmstrip},
    player::{CheckKind, Outcome},
    recorder::{Capture, RecordError, ScenarioRecording, create_dir, write_file},
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

/// Writes a scenario's pictures, filmstrip and GIF into `folder`.
pub(crate) fn write_scenario(folder: &Path, pictures: &[Capture]) -> Result<(), RecordError> {
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
    Ok(())
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
    let class = if scenario.passed() { "pass" } else { "fail" };
    let verdict = if scenario.passed() {
        "passed"
    } else {
        "FAILED"
    };
    let slug = escape(&scenario.slug);
    let _ = write!(
        out,
        "<article class=\"card {class}\" id=\"{slug}\"><h3>{name} <small>{verdict}</small></h3>\
         <a href=\"{slug}/{FILMSTRIP_FILE}\"><img src=\"{slug}/{GIF_FILE}\" alt=\"{name}\" \
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
    let _ = write!(
        out,
        "</ul><p><a href=\"{slug}/{FILMSTRIP_FILE}\">filmstrip</a> \u{b7} \
         <a href=\"{slug}/\">{} pictures</a></p></article>",
        scenario.pictures
    );
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
        members.sort_by_key(|scenario| scenario.passed());
    }
    groups.sort_by_key(|(_, members)| members.iter().all(|scenario| scenario.passed()));

    let failed = scenarios
        .iter()
        .filter(|scenario| !scenario.passed())
        .count();
    let mut out = String::from(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Scenario recordings</title><style>\
         body{font-family:sans-serif;background:#18181c;color:#eee;margin:1.5rem}\
         .grid{display:flex;flex-wrap:wrap;gap:1rem}\
         .card{width:340px;background:#24242c;border-left:6px solid #3c9a54;padding:.5rem .8rem}\
         .card.fail{border-left-color:#c84040}\
         .card img{width:100%}\
         .card h3{margin:.2rem 0}.card small{font-weight:normal;opacity:.7}\
         ul{list-style:none;padding:0;margin:.4rem 0;font-size:.9rem}\
         li.pass{color:#7fdc8c}li.fail{color:#ff7a7a}li.skipped{color:#999}\
         a{color:#9cc8ff}</style></head><body><h1>Scenario recordings</h1>",
    );
    let _ = write!(
        out,
        "<p>{} scenarios, {failed} failed.</p>",
        scenarios.len()
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
    use crate::player::ScenarioReport;

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
            pictures: 3,
        }
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
