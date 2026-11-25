//! Pick mode: outlines the node under the real pointer in the app window and
//! picks it on click. Writes only the gallery's state and [`PickHighlight`].

use bevy::ecs::{entity::Entity, system::Local, world::World};

use super::{
    input::{PickClicks, RealCursor},
    state::GalleryState,
};
use crate::{
    check::{
        Check,
        pick::{Picked, node_at, subject_of, target_alternatives},
    },
    overlay::PickHighlight,
    session::{SessionConfig, SessionHooks},
};

pub(super) fn spelled(check: &Check) -> String {
    ron::to_string(check).unwrap_or_else(|_| check.to_string())
}

pub(super) fn update(world: &mut World, mut tagged: Local<Option<(Entity, String)>>) {
    let clicks = world
        .get_resource_mut::<PickClicks>()
        .map_or(0, |mut clicks| core::mem::take(&mut clicks.0));
    let picking = world
        .get_resource::<GalleryState>()
        .is_some_and(GalleryState::is_picking);
    if !picking {
        *tagged = None;
        set_highlight(world, PickHighlight::default());
        return;
    }
    let position = world
        .get_resource::<RealCursor>()
        .and_then(RealCursor::position);
    let hit = position.and_then(|position| node_at(world, position));
    let subject = hit.map(|node| subject_of(world, node));
    // Ranking the targets reads every text of the app: only on a new node.
    let tag = match (subject, tagged.as_ref()) {
        (Some(subject), Some((was, tag))) if *was == subject => tag.clone(),
        (Some(subject), _) => target_alternatives(world, Some(subject), position)
            .first()
            .map_or_default(|target| format!("{target:?}")),
        (None, _) => String::new(),
    };
    *tagged = subject.map(|subject| (subject, tag.clone()));
    if clicks > 0
        && let (Some(node), Some(position)) = (hit, position)
    {
        let hooks = world
            .get_resource::<SessionConfig>()
            .map_or_else(SessionHooks::default, |config| config.hooks.clone());
        let picked = Picked::new(world, node, hooks.noise(), Some(position));
        if let Some(mut state) = world.get_resource_mut::<GalleryState>() {
            state.status = picked_status(&picked, state.is_recording());
            state.picked = Some(picked);
        }
    }
    let selected = world
        .get_resource::<GalleryState>()
        .and_then(GalleryState::picked)
        .map(Picked::subject);
    set_highlight(
        world,
        PickHighlight {
            hovered: subject,
            selected,
            tag,
        },
    );
}

fn picked_status(picked: &Picked, recording: bool) -> String {
    let target = picked
        .target()
        .map_or_else(|| "a node".to_owned(), |target| format!("{target:?}"));
    let place = if recording {
        "the recording, here"
    } else {
        "the scenario's expect"
    };
    let checks = match picked.checks().len() {
        0 => "no built-in check holds".to_owned(),
        1 => "1 check holds".to_owned(),
        many => format!("{many} checks hold"),
    };
    format!("Picked {target}: {checks}; click one to add it to {place}")
}

fn set_highlight(world: &mut World, wanted: PickHighlight) {
    if let Some(mut highlight) = world.get_resource_mut::<PickHighlight>()
        && *highlight != wanted
    {
        *highlight = wanted;
    }
}
