//! Widget roles, read from the components Bevy's own widgets keep.
//!
//! Roles come from `bevy_ui_widgets`, not AccessKit: the accessibility tree
//! exists only where a backend runs.

use bevy::{
    ecs::world::EntityRef,
    ui_widgets::{
        Button, Checkbox, ListItem, MenuButton, MenuItem, RadioButton, Slider, Tab, TabList,
        TextInput,
    },
};
use serde::{Deserialize, Serialize};

/// The kind of widget a [`Target::Role`](super::Target::Role) looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WidgetRole {
    Button,
    Checkbox,
    RadioButton,
    Slider,
    MenuButton,
    MenuItem,
    ListItem,
    Tab,
    TabList,
    /// Its label is the text beside it, not its value: read that with `TextOf` or `Shows`.
    TextInput,
}

impl WidgetRole {
    /// Most specific first: a menu button is a button too.
    pub const ALL: [Self; 10] = [
        Self::MenuButton,
        Self::Checkbox,
        Self::RadioButton,
        Self::Slider,
        Self::MenuItem,
        Self::ListItem,
        Self::Tab,
        Self::TabList,
        Self::TextInput,
        Self::Button,
    ];

    #[must_use]
    pub fn fits(self, entity: &EntityRef<'_>) -> bool {
        match self {
            Self::Button => entity.contains::<Button>(),
            Self::Checkbox => entity.contains::<Checkbox>(),
            Self::RadioButton => entity.contains::<RadioButton>(),
            Self::Slider => entity.contains::<Slider>(),
            Self::MenuButton => entity.contains::<MenuButton>(),
            Self::MenuItem => entity.contains::<MenuItem>(),
            Self::ListItem => entity.contains::<ListItem>(),
            Self::Tab => entity.contains::<Tab>(),
            Self::TabList => entity.contains::<TabList>(),
            Self::TextInput => entity.contains::<TextInput>(),
        }
    }

    #[must_use]
    pub fn of(entity: &EntityRef<'_>) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.fits(entity))
    }

    /// Where on the widget a pointer lands decides what it does, as on a slider track.
    #[must_use]
    pub const fn is_positional(self) -> bool {
        matches!(self, Self::Slider)
    }
}
