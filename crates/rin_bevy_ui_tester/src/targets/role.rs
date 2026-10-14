//! Widget roles: what kind of widget a node is, read from the components
//! Bevy's own widgets keep.
//!
//! Roles come from `bevy_ui_widgets`, not from
//! AccessKit: the widget components are there in every app that uses the
//! widgets, headless or not, while the accessibility tree is only built where
//! an accessibility backend runs.

use bevy::{
    ecs::world::EntityRef,
    ui_widgets::{Button, Checkbox, ListItem, MenuButton, MenuItem, RadioButton, Slider},
};
use serde::{Deserialize, Serialize};

/// The kind of widget a [`Target::Role`](super::Target::Role) looks for,
/// like an ARIA role in Playwright's `getByRole`.
///
/// Each role is the component a widget of `bevy_ui_widgets` carries:
///
/// | Role          | Component                                           |
/// | ------------- | --------------------------------------------------- |
/// | `Button`      | `bevy_ui_widgets::Button`                           |
/// | `Checkbox`    | `Checkbox`                                          |
/// | `RadioButton` | `RadioButton`                                       |
/// | `Slider`      | `Slider`                                            |
/// | `MenuButton`  | `MenuButton` (which is a `Button` as well)          |
/// | `MenuItem`    | `MenuItem`                                          |
/// | `ListItem`    | `ListItem`                                          |
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WidgetRole {
    /// A button.
    Button,
    /// A checkbox, or a toggle built on one.
    Checkbox,
    /// One button of a radio group.
    RadioButton,
    /// A slider.
    Slider,
    /// A button that opens a menu.
    MenuButton,
    /// An item in a menu.
    MenuItem,
    /// An item in a list box.
    ListItem,
}

impl WidgetRole {
    /// Every role, the most specific first: a menu button is a button too,
    /// and is named by the more specific role.
    pub const ALL: [Self; 7] = [
        Self::MenuButton,
        Self::Checkbox,
        Self::RadioButton,
        Self::Slider,
        Self::MenuItem,
        Self::ListItem,
        Self::Button,
    ];

    /// Whether `entity` is a widget of this role.
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
        }
    }

    /// The most specific role `entity` has, if it is a widget of any.
    #[must_use]
    pub fn of(entity: &EntityRef<'_>) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.fits(entity))
    }

    /// Whether where on the widget a pointer lands decides what it does, as
    /// on a slider's track; a recorded click on one keeps its position.
    #[must_use]
    pub const fn is_positional(self) -> bool {
        matches!(self, Self::Slider)
    }
}
