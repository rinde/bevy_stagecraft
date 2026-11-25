//! Turns a panic in the app under test into a failed step.
//!
//! Caught errors live in a thread local because a handler is a plain function.

use alloc::{format, string::String, vec::Vec};
use core::cell::RefCell;

use bevy::ecs::{
    error::{
        BevyError, ErrorContext, ErrorHandler, FallbackErrorHandler, Severity, match_severity,
    },
    world::World,
};

thread_local! {
    static CAUGHT: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

fn catch(mut error: BevyError, ctx: ErrorContext) {
    if matches!(error.severity(), Severity::Panic) {
        let payload = error.take_payload();
        let detail = payload
            .as_ref()
            .and_then(|payload| {
                payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
            })
            .map_or_else(String::new, |text| format!(": {text}"));
        let kept = CAUGHT.with(|caught| {
            caught.borrow_mut().as_mut().map(|caught| {
                caught.push(format!("{ctx}{detail}"));
            })
        });
        if kept.is_some() {
            return;
        }
    }
    match_severity(error, ctx);
}

/// Returns the handler it replaces.
pub(super) fn install(world: &mut World) -> ErrorHandler {
    CAUGHT.with(|caught| *caught.borrow_mut() = Some(Vec::new()));
    let mut fallback = world.get_resource_or_insert_with(FallbackErrorHandler::default);
    let previous = fallback.0;
    fallback.0 = catch;
    previous
}

pub(super) fn restore(world: &mut World, previous: ErrorHandler) {
    CAUGHT.with(|caught| *caught.borrow_mut() = None);
    world.insert_resource(FallbackErrorHandler(previous));
}

pub(super) fn take() -> Vec<String> {
    CAUGHT.with(|caught| caught.borrow_mut().as_mut().map_or_default(core::mem::take))
}
