#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken setup"
)]

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use bevy::{
    asset::{
        AssetApp, AssetLoader, LoadContext,
        io::{
            AssetSourceBuilder, AssetSourceId, Reader,
            memory::{Dir, MemoryAssetReader},
        },
    },
    ecs::template::FromTemplate,
    prelude::*,
    reflect::TypePath,
    scene::{ScenePatch, ScenePlugin, WorldSceneExt, bsn},
};
use bevy_stagecraft::{Check, HeadlessPlugins, Scenario, Step};

#[derive(Component, Reflect, Default, Clone, FromTemplate)]
#[reflect(Component)]
struct Inner;

#[derive(Component, Reflect, Default, Clone, FromTemplate)]
#[reflect(Component)]
struct Outer;

#[derive(Default)]
struct Gate {
    frames: u32,
    waker: Option<Waker>,
}

type SharedGate = Arc<Mutex<Gate>>;

#[derive(TypePath)]
struct SlowSceneLoader {
    gate: SharedGate,
    release_at: u32,
}

struct UntilFrame {
    gate: SharedGate,
    release_at: u32,
}

impl Future for UntilFrame {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut gate = self.gate.lock().expect("the gate is not poisoned");
        if gate.frames >= self.release_at {
            Poll::Ready(())
        } else {
            gate.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl AssetLoader for SlowSceneLoader {
    type Asset = ScenePatch;
    type Error = std::io::Error;
    type Settings = ();

    async fn load(
        &self,
        _reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        UntilFrame {
            gate: self.gate.clone(),
            release_at: self.release_at,
        }
        .await;
        Ok(ScenePatch::load_with(load_context, bsn! { Inner }))
    }
}

fn app(release_at: u32) -> App {
    let dir = Dir::default();
    dir.insert_asset_text(Path::new("inner.bsn"), "");
    let gate = SharedGate::default();
    let mut app = App::new();
    app.register_asset_source(
        AssetSourceId::Default,
        AssetSourceBuilder::new(move || Box::new(MemoryAssetReader { root: dir.clone() })),
    );
    app.add_plugins((HeadlessPlugins::default(), ScenePlugin));
    app.register_asset_loader(SlowSceneLoader {
        gate: gate.clone(),
        release_at,
    });
    app.add_systems(First, move || {
        let mut gate = gate.lock().expect("the gate is not poisoned");
        gate.frames += 1;
        if gate.frames >= release_at
            && let Some(waker) = gate.waker.take()
        {
            waker.wake();
        }
    });
    app
}

fn spawn_outer(world: &mut World) {
    world.queue_spawn_scene(bsn! { :"inner.bsn" Outer });
}

#[test]
fn the_first_check_passes_without_within_once_the_nested_scene_asset_is_loaded() {
    let mut app = app(30);
    let report = Scenario::new("nested scene")
        .setup(spawn_outer)
        .steps([
            Step::check(Check::count::<Outer>().eq(1)),
            Step::check(Check::count::<Inner>().eq(1)),
        ])
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    assert!(
        report.frames >= 28,
        "the first step ran after {} frames, before the asset loaded",
        report.frames
    );
}

#[test]
fn a_scene_that_never_loads_is_a_setup_failure_naming_its_asset() {
    let mut app = app(u32::MAX);
    let report = Scenario::new("stuck scene")
        .setup(spawn_outer)
        .action_timeout(5)
        .step(Step::check(Check::count::<Outer>().eq(1)))
        .run_in(&mut app);
    assert!(!report.passed());
    let failure = report.setup_failure.expect("a setup failure");
    assert!(failure.contains("inner.bsn"), "{failure}");
    assert!(failure.contains("after 5 frames"), "{failure}");
    assert!(report.finished);
}

#[test]
fn a_setup_that_spawns_no_scene_does_not_wait() {
    let mut app = app(u32::MAX);
    let report = Scenario::new("no scene")
        .step(Step::check(Check::count::<Outer>().eq(0)))
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    assert!(report.frames < 10, "{} frames", report.frames);
}

#[test]
fn a_scene_without_assets_is_ready_at_once() {
    let mut app = app(u32::MAX);
    let report = Scenario::new("plain scene")
        .setup(|world| {
            world
                .spawn_scene(bsn! { Outer })
                .expect("a scene without assets spawns at once");
        })
        .step(Step::check(Check::count::<Outer>().eq(1)))
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
}
