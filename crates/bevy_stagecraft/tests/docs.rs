//! The documentation budgets: `AGENTS.md`, `README.md` and `agent-docs/` stay
//! small, linked and free of dead links.
//!
//! The limits below are the source of truth; `agent-docs/writing-docs.md`
//! names this file for them. A failure means split the file by topic, not
//! compress it.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken setup"
)]

use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::{Component, Path, PathBuf},
};

/// Lines `AGENTS.md` may have.
const AGENTS_MAX_LINES: usize = 100;
/// Bytes `AGENTS.md` may have.
const AGENTS_MAX_BYTES: usize = 6 * 1024;
/// Lines `README.md` may have.
const README_MAX_LINES: usize = 200;
/// Bytes `README.md` may have.
const README_MAX_BYTES: usize = 10 * 1024;
/// Lines each markdown file under `agent-docs/` may have.
const GUIDE_MAX_LINES: usize = 250;
/// Bytes each markdown file under `agent-docs/` may have.
const GUIDE_MAX_BYTES: usize = 16 * 1024;
/// Characters a line may have outside fenced code blocks.
const MAX_LINE_CHARS: usize = 200;
/// What to do about a budget failure.
const ADVICE: &str = "split it by topic (do not compress it), see agent-docs/writing-docs.md";

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every markdown file under `dir`, recursively, as paths relative to the root.
fn markdown_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root().join(dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let rel = dir.join(entry.file_name());
        if entry.path().is_dir() {
            markdown_under(&rel, out);
        } else if rel.extension().is_some_and(|e| e == "md") {
            out.push(rel);
        }
    }
}

/// The checked files: `AGENTS.md`, `README.md` and `agent-docs/**/*.md`.
fn doc_files() -> Vec<PathBuf> {
    let mut files = vec![PathBuf::from("AGENTS.md"), PathBuf::from("README.md")];
    markdown_under(Path::new("agent-docs"), &mut files);
    files.sort();
    files
}

/// The text of a file relative to the root.
fn read(rel: &Path) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_default()
}

/// The lines of `text` outside fenced code blocks, with their 1-based number.
fn prose_lines(text: &str) -> Vec<(usize, &str)> {
    let mut fenced = false;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced {
            out.push((i + 1, line));
        }
    }
    out
}

/// The targets of the inline links `[text](target)` on a line.
fn link_targets(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("](") {
        let after = rest.get(at + 2..).unwrap_or_default();
        let Some(end) = after.find(')') else { break };
        if let Some(target) = after.get(..end) {
            out.push(target.split_whitespace().next().unwrap_or_default());
        }
        rest = after.get(end..).unwrap_or_default();
    }
    out
}

/// The GitHub anchor of a heading text.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The anchors a markdown file offers.
fn anchors(text: &str) -> BTreeSet<String> {
    prose_lines(text)
        .into_iter()
        .filter_map(|(_, l)| l.trim_start().strip_prefix('#'))
        .map(|h| slug(h.trim_start_matches('#')))
        .collect()
}

/// `base` joined with `target`, with `.` and `..` resolved.
fn resolve(base: &Path, target: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for c in base
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(target)
        .components()
    {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Fails, listing every problem, when there are any.
fn assert_clean(problems: &[String]) {
    assert!(
        problems.is_empty(),
        "docs check failed:\n- {}",
        problems.join("\n- ")
    );
}

#[test]
fn docs_stay_within_their_budgets() {
    let mut problems = Vec::new();
    for file in doc_files() {
        let (max_lines, max_bytes) = match file.to_str() {
            Some("AGENTS.md") => (AGENTS_MAX_LINES, AGENTS_MAX_BYTES),
            Some("README.md") => (README_MAX_LINES, README_MAX_BYTES),
            _ => (GUIDE_MAX_LINES, GUIDE_MAX_BYTES),
        };
        let text = read(&file);
        let (lines, bytes) = (text.lines().count(), text.len());
        let name = file.display();
        if lines > max_lines {
            problems.push(format!(
                "{name} has {lines} lines, the limit is {max_lines}: {ADVICE}"
            ));
        }
        if bytes > max_bytes {
            problems.push(format!(
                "{name} is {bytes} bytes, the limit is {max_bytes}: {ADVICE}"
            ));
        }
    }
    assert_clean(&problems);
}

#[test]
fn doc_lines_stay_short() {
    let mut problems = Vec::new();
    for file in doc_files() {
        let text = read(&file);
        for (n, line) in prose_lines(&text) {
            let chars = line.chars().count();
            if chars > MAX_LINE_CHARS {
                problems.push(format!(
                    "{}:{n} has {chars} characters, the limit is {MAX_LINE_CHARS}: \
                     break it up, see agent-docs/writing-docs.md",
                    file.display()
                ));
            }
        }
    }
    assert_clean(&problems);
}

#[test]
fn doc_links_and_anchors_resolve() {
    let mut problems = Vec::new();
    for file in doc_files() {
        let text = read(&file);
        for (n, line) in prose_lines(&text) {
            for target in link_targets(line) {
                if target.contains("://") || target.starts_with("mailto:") || target.is_empty() {
                    continue;
                }
                let (path, anchor) = target.split_once('#').unwrap_or((target, ""));
                let dest = if path.is_empty() {
                    file.clone()
                } else {
                    resolve(&file, path)
                };
                if !root().join(&dest).exists() {
                    problems.push(format!(
                        "{}:{n} links to {target}, which does not exist",
                        file.display()
                    ));
                } else if !anchor.is_empty()
                    && dest.extension().is_some_and(|e| e == "md")
                    && !anchors(&read(&dest)).contains(anchor)
                {
                    problems.push(format!(
                        "{}:{n} links to {target}, but {} has no such heading",
                        file.display(),
                        dest.display()
                    ));
                }
            }
        }
    }
    assert_clean(&problems);
}

#[test]
fn every_agent_doc_is_linked() {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: VecDeque<PathBuf> =
        VecDeque::from([PathBuf::from("AGENTS.md"), PathBuf::from("README.md")]);
    while let Some(file) = queue.pop_front() {
        if !seen.insert(file.clone()) {
            continue;
        }
        let text = read(&file);
        for (_, line) in prose_lines(&text) {
            for target in link_targets(line) {
                let path = target.split('#').next().unwrap_or_default();
                if Path::new(path).extension().is_some_and(|e| e == "md") && !path.contains("://") {
                    queue.push_back(resolve(&file, path));
                }
            }
        }
    }
    let problems: Vec<String> = doc_files()
        .into_iter()
        .filter(|f| f.starts_with("agent-docs") && !seen.contains(f))
        .map(|f| {
            format!(
                "{} is not linked from AGENTS.md, README.md or a file reachable from them: \
                 link it from the index in AGENTS.md, or delete it",
                f.display()
            )
        })
        .collect();
    assert_clean(&problems);
}

#[test]
fn the_temporary_module_details_file_is_gone() {
    assert!(
        !root().join("agent-docs/module-details.md").exists(),
        "agent-docs/module-details.md was a temporary file: move its content into rustdoc or a topic file and delete it"
    );
}
