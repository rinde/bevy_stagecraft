//! Turns a panic in the app under test into a failed step.
//!
//! Since Bevy 0.20 a panic in a system, command or observer reaches the
//! world's [`FallbackErrorHandler`] as an error, and the default handler
//! panics again. While a scenario plays, the player swaps in a handler that
//! keeps such errors, and puts the app's own handler back afterwards.
//!
//! A handler is a plain function, so what it keeps lives in a thread local:
//! the apps this crate plays run their schedules on the thread that updates
//! them, and tests running in parallel threads never see each other's
//! errors. An error that arrives on a thread that is not catching goes to
//! Bevy's default handler, as it did before.

use alloc::{format, string::String, vec::Vec};
use core::cell::RefCell;

use bevy::ecs::{
    error::{
        BevyError, ErrorContext, ErrorHandler, FallbackErrorHandler, Severity, match_severity,
    },
    world::World,
};

thread_local! {
    /// The errors kept since the last [`take`], or `None` while this thread
    /// is not catching.
    static CAUGHT: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// The handler a playing scenario installs: keeps a panic or an error of
/// severity [`Severity::Panic`] while this thread is catching, and leaves
/// everything else to Bevy.
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

/// Starts catching on this thread and installs the handler in `world`,
/// returning the handler it replaces.
pub(super) fn install(world: &mut World) -> ErrorHandler {
    CAUGHT.with(|caught| *caught.borrow_mut() = Some(Vec::new()));
    let mut fallback = world.get_resource_or_insert_with(FallbackErrorHandler::default);
    let previous = fallback.0;
    fallback.0 = catch;
    previous
}

/// Stops catching on this thread and puts `previous` back in `world`.
pub(super) fn restore(world: &mut World, previous: ErrorHandler) {
    CAUGHT.with(|caught| *caught.borrow_mut() = None);
    world.insert_resource(FallbackErrorHandler(previous));
}

/// The errors kept since the last call, oldest first.
pub(super) fn take() -> Vec<String> {
    CAUGHT.with(|caught| {
        caught
            .borrow_mut()
            .as_mut()
            .map(core::mem::take)
            .unwrap_or_default()
    })
}
