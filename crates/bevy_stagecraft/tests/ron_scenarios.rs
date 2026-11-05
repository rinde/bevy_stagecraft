//! Every `.ron` file in `tests/ron` is one test, named after the file.

mod common;

use std::process::ExitCode;

fn main() -> ExitCode {
    let registry = common::registry();
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ron");
    bevy_stagecraft::run_ron_tests(dir, &registry, |scenario| {
        let mut app = scenario.headless_app();
        registry.install(app.world_mut());
        scenario.run_in(&mut app)
    })
}
