//! Tags and annotations of a scenario, as Playwright's `@tag`, `skip`,
//! `fixme` and `fail`.
//!
//! A scenario carries `tags`, and may be skipped (`skip`), set aside as a
//! known problem (`fixme`) or expected to fail (`should_fail`); see
//! [`crate::Scenario::tags`]. A runner chooses scenarios by tag with a
//! [`TagFilter`], which `run_ron_tests` reads from the environment variable
//! [`TAGS_ENV`].

/// The environment variable `run_ron_tests` reads its [`TagFilter`] from:
/// `RIN_SCENARIO_TAGS=bridge,-slow` plays the scenarios tagged `bridge`
/// except those tagged `slow`.
pub const TAGS_ENV: &str = "RIN_SCENARIO_TAGS";

/// Why a runner leaves a scenario out: it is skipped, or marked fixme. Both
/// carry the reason; the two are listed apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disabled {
    /// `skip: Some(reason)`: not run.
    Skip(String),
    /// `fixme: Some(reason)`: a known problem, not run, listed separately.
    Fixme(String),
}

impl Disabled {
    /// The reason the scenario gave.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Skip(reason) | Self::Fixme(reason) => reason,
        }
    }

    /// The word a runner puts before the reason: `skip` or `fixme`.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Skip(_) => "skip",
            Self::Fixme(_) => "fixme",
        }
    }

    /// The reason with its label, `fixme: not built yet`.
    #[must_use]
    pub fn describe(&self) -> String {
        format!("{}: {}", self.label(), self.reason())
    }
}

/// Which tags a runner plays: a comma-separated list where `bridge` requires
/// a tag and `-slow` forbids one.
///
/// A scenario matches when it has none of the forbidden tags and, if any tag
/// is required, at least one of them. An empty filter matches everything.
///
/// ```
/// use bevy_stagecraft::TagFilter;
///
/// let filter = TagFilter::parse("bridge,-slow");
/// assert!(filter.matches(&["bridge".to_owned()]));
/// assert!(!filter.matches(&["bridge".to_owned(), "slow".to_owned()]));
/// assert!(!filter.matches(&[]));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagFilter {
    /// Tags of which a scenario needs at least one; empty needs none.
    include: Vec<String>,
    /// Tags a scenario must not have.
    exclude: Vec<String>,
}

impl TagFilter {
    /// Reads `bridge,-slow`: words split at commas, blank ones ignored.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut filter = Self::default();
        for word in text.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            match word.strip_prefix('-') {
                Some(tag) if !tag.is_empty() => filter.exclude.push(tag.to_owned()),
                Some(_) => {}
                None => filter.include.push(word.to_owned()),
            }
        }
        filter
    }

    /// The filter in [`TAGS_ENV`]; everything where it is not set.
    #[must_use]
    pub fn from_env() -> Self {
        std::env::var(TAGS_ENV).map_or_else(|_| Self::default(), |text| Self::parse(&text))
    }

    /// Whether the filter lets everything through.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// Whether a scenario with these tags is played.
    #[must_use]
    pub fn matches(&self, tags: &[String]) -> bool {
        let has = |wanted: &String| tags.contains(wanted);
        !self.exclude.iter().any(has) && (self.include.is_empty() || self.include.iter().any(has))
    }

    /// The filter as written, for a status line: `bridge,-slow`.
    #[must_use]
    pub fn describe(&self) -> String {
        let excluded = self.exclude.iter().map(|tag| format!("-{tag}"));
        self.include
            .iter()
            .cloned()
            .chain(excluded)
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn an_empty_filter_matches_every_scenario() {
        let filter = TagFilter::parse(" , ");
        assert!(filter.is_empty());
        assert!(filter.matches(&tags(&[])));
        assert!(filter.matches(&tags(&["slow"])));
    }

    #[test]
    fn a_required_tag_must_be_present() {
        let filter = TagFilter::parse("bridge");
        assert!(filter.matches(&tags(&["bridge", "slow"])));
        assert!(!filter.matches(&tags(&["slow"])));
    }

    #[test]
    fn only_forbidden_tags_let_untagged_scenarios_through() {
        let filter = TagFilter::parse("-slow");
        assert!(filter.matches(&tags(&[])));
        assert!(!filter.matches(&tags(&["slow"])));
    }

    #[test]
    fn a_filter_describes_itself_as_written() {
        assert_eq!(TagFilter::parse("bridge, -slow").describe(), "bridge,-slow");
    }
}
