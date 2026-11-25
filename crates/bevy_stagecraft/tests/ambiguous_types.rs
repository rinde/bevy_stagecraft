#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::panic,
    unreachable_pub,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    reflect::TypePath,
    ui::px,
    ui_widgets::{Activate, Button},
};
use bevy_stagecraft::{HeadlessPlugins, LoadError, Outcome, Registry, Scenario};

mod one {
    use bevy::prelude::*;

    #[derive(Component, Reflect, Debug, Default)]
    #[reflect(Component)]
    pub struct Label;

    #[derive(Resource, Reflect, Debug, Default)]
    #[reflect(Resource)]
    pub struct Score(pub u32);
}

mod two {
    use bevy::prelude::*;

    #[derive(Component, Reflect, Debug, Default)]
    #[reflect(Component)]
    pub struct Label;

    #[derive(Resource, Reflect, Debug, Default)]
    #[reflect(Resource)]
    pub struct Score(pub u32);
}

#[derive(Resource, Default, Debug)]
struct Activations(u32);

fn path<T: TypePath>() -> &'static str {
    T::type_path()
}

fn load(step: &str) -> Result<Scenario, LoadError> {
    Scenario::from_ron_with(
        &format!("(format_version: 3, name: \"x\", steps: [{step}])"),
        &Registry::default(),
    )
}

#[test]
fn a_short_path_shared_by_two_types_is_refused_listing_both_full_paths() {
    let listed = format!(
        "`Label` names more than one registered type (`{}`, `{}`, `{}`); write its full type path",
        path::<one::Label>(),
        path::<two::Label>(),
        path::<Label>()
    );
    for step in [
        "MoveTo(Component(\"Label\"))",
        "Check(Count(with: [\"Label\"], is: Eq(1)))",
        "Check(Component(target: Component(\"Label\"), path: \"Label\", is: Eq(Label)))",
    ] {
        let error = load(step).unwrap_err();
        assert!(
            error.to_string().contains(&listed),
            "{step}: {error}\nexpected {listed}"
        );
    }
    let error = load("Check(Resource(path: \"Score.0\", is: Eq(0)))").unwrap_err();
    assert!(
        error.to_string().contains(&format!(
            "`Score` names more than one registered type (`{}`, `{}`)",
            path::<one::Score>(),
            path::<two::Score>()
        )),
        "{error}"
    );
}

#[test]
fn a_full_type_path_is_accepted_wherever_a_short_one_is() {
    let (label, other) = (path::<two::Label>(), path::<one::Label>());
    let score = path::<two::Score>();
    for step in [
        format!("MoveTo(Component(\"{label}\"))"),
        format!("Check(Exists(Within(Component(\"{other}\"), Component(\"{label}\"))))"),
        format!("Check(Count(with: [\"{label}\"], is: Eq(1)))"),
        format!("Check(Resource(path: \"{score}.0\", is: Eq(0)))"),
    ] {
        assert!(load(&step).is_ok(), "{step}: {:?}", load(&step).err());
    }
}

#[test]
fn a_target_by_full_type_path_finds_the_entity_of_that_type_only() {
    let script = format!(
        "(format_version: 3, name: \"full path\", steps: [\
         MoveTo(Component(\"{label}\")), Click(Left), Wait(1), \
         Check(Count(with: [\"{label}\"], is: Eq(1))), \
         Check(Count(with: [\"{other}\"], is: Eq(1))), \
         Check(Resource(path: \"{score}.0\", is: Eq(7)))])",
        label = path::<two::Label>(),
        other = path::<one::Label>(),
        score = path::<two::Score>(),
    );
    let button = |top: i32| {
        (
            Button,
            Node {
                position_type: PositionType::Absolute,
                top: px(top),
                width: px(100),
                height: px(30),
                ..Node::default()
            },
        )
    };
    let scenario = Scenario::from_ron_with(&script, &Registry::default())
        .expect("full type paths load")
        .setup(move |world| {
            world.init_resource::<Activations>();
            world.insert_resource(two::Score(7));
            world.spawn(Camera2d);
            world.spawn((button(0), one::Label));
            world.spawn((button(100), two::Label)).observe(
                |_: On<Activate>, mut activations: ResMut<Activations>| activations.0 += 1,
            );
        });
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    let report = scenario.run_in(&mut app);
    assert!(
        report
            .steps
            .iter()
            .all(|record| record.outcome == Outcome::Pass),
        "{:?}",
        report.failures()
    );
    assert_eq!(app.world().resource::<Activations>().0, 1);
}
