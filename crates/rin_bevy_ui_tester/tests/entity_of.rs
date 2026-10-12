//! `entity_of`: the entity a target names, for an app check or target that
//! must reach one with no `Node`, such as a selection shared between widgets.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{prelude::*, ui_widgets::Button};
use rin_bevy_ui_tester::{HeadlessPlugins, NotUnderTest, Target, WidgetRole, entity_of, resolve};

/// The marker of the bare selection entity.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct SharedSelection;

/// Marks both bare entities holding a value, so it names neither.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct Holder;

/// A headless app with nothing spawned.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    app
}

#[test]
fn a_component_locator_finds_a_bare_entity_without_a_node() {
    let mut app = app();
    let bare = app.world_mut().spawn(SharedSelection).id();
    app.world_mut().spawn((Button, Node::default()));
    let world = app.world();
    assert_eq!(
        entity_of(world, &Target::component::<SharedSelection>()),
        Ok(bare)
    );
    let message =
        resolve(world, &Target::component::<SharedSelection>()).expect_err("a pixel needs a Node");
    assert!(message.ends_with("which is not a UI node"), "{message}");
}

#[test]
fn a_name_locator_finds_a_bare_entity_without_a_node() {
    let mut app = app();
    let bare = app.world_mut().spawn(Name::new("selection")).id();
    assert_eq!(entity_of(app.world(), &Target::name("selection")), Ok(bare));
}

#[test]
fn entity_of_returns_the_entity_itself_not_the_button_around_it() {
    let mut app = app();
    let label = app.world_mut().spawn(Name::new("label")).id();
    app.world_mut()
        .spawn((Button, Node::default()))
        .add_child(label);
    assert_eq!(entity_of(app.world(), &Target::name("label")), Ok(label));
}

#[test]
fn nth_and_within_pick_between_bare_entities() {
    let mut app = app();
    // Matches are counted in hierarchy order, so the holders are siblings.
    let list = app.world_mut().spawn_empty().id();
    let first = app.world_mut().spawn((Holder, ChildOf(list))).id();
    let second = app.world_mut().spawn((Holder, ChildOf(list))).id();
    let owner = app.world_mut().spawn(Name::new("owner")).id();
    let inside = app.world_mut().spawn(SharedSelection).id();
    app.world_mut().entity_mut(owner).add_child(inside);
    let world = app.world();
    let holders = Target::component::<Holder>();
    let message = entity_of(world, &holders).expect_err("two match");
    assert!(
        message.contains("is ambiguous: it matches 2 widgets"),
        "{message}"
    );
    assert_eq!(
        entity_of(world, &Target::nth(0, holders.clone())),
        Ok(first)
    );
    assert_eq!(entity_of(world, &Target::nth(1, holders)), Ok(second));
    assert_eq!(
        entity_of(
            world,
            &Target::within(
                Target::name("owner"),
                Target::component::<SharedSelection>()
            )
        ),
        Ok(inside)
    );
}

#[test]
fn entities_of_the_tooling_are_not_found() {
    let mut app = app();
    app.world_mut().spawn((Name::new("hidden"), NotUnderTest));
    let message = entity_of(app.world(), &Target::name("hidden")).expect_err("tooling is skipped");
    assert!(
        message.contains("no entity is named \"hidden\""),
        "{message}"
    );
}

#[test]
fn entity_of_says_why_a_target_names_no_entity() {
    let mut app = app();
    app.world_mut().spawn(Name::new("selection"));
    let world = app.world();
    let missing = entity_of(world, &Target::name("selectoin")).expect_err("a typo");
    assert!(missing.contains("did you mean \"selection\"?"), "{missing}");
    let nothing = entity_of(world, &Target::component::<SharedSelection>()).expect_err("none");
    assert!(nothing.contains("no entity has a `"), "{nothing}");
    let position = entity_of(world, &Target::screen(Vec2::ZERO)).expect_err("a position");
    assert!(
        position.contains("is a position, not a widget"),
        "{position}"
    );
    let role = entity_of(world, &Target::of_role(WidgetRole::Button)).expect_err("no button");
    assert!(role.contains("there is no"), "{role}");
}
