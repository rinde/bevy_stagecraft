/// Holds the [`TagFilter`] text, such as `bridge,-slow`.
pub const TAGS_ENV: &str = "RIN_SCENARIO_TAGS";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disabled {
    Skip(String),
    Fixme(String),
}

impl Disabled {
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Skip(reason) | Self::Fixme(reason) => reason,
        }
    }

    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Skip(_) => "skip",
            Self::Fixme(_) => "fixme",
        }
    }

    #[must_use]
    pub fn describe(&self) -> String {
        format!("{}: {}", self.label(), self.reason())
    }
}

/// Comma-separated tags: `bridge` requires one, `-slow` forbids one.
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
    include: Vec<String>,
    exclude: Vec<String>,
}

impl TagFilter {
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

    #[must_use]
    pub fn from_env() -> Self {
        std::env::var(TAGS_ENV).map_or_else(|_| Self::default(), |text| Self::parse(&text))
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    #[must_use]
    pub fn matches(&self, tags: &[String]) -> bool {
        let has = |wanted: &String| tags.contains(wanted);
        !self.exclude.iter().any(has) && (self.include.is_empty() || self.include.iter().any(has))
    }

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
