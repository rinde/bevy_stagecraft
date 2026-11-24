//! Reading scenario files again when they change (see
//! [`Gallery::directory`](super::Gallery::directory)).
//!
//! [`reload_changed_files`] runs every [`WATCH_INTERVAL`] updates, never during
//! a run of everything. A good reload forgets that entry's results
//! (`GalleryState::forget_results`) and, if the entry is selected and nothing
//! is being recorded, asks for a `Restart`. An added entry shifts the selection
//! and the results (`GalleryState::insert_entry`). A file that does not load
//! leaves the entry as it was and puts the reason in the status line. Every
//! change bumps the gallery's `revision`, on which
//! `ui::rebuild_catalogue` respawns the catalogue, with an error shown under
//! its entry.

use bevy::ecs::system::{Local, ResMut};

use super::{Gallery, GalleryAction, GalleryState, Reload, catalogue::WATCH_INTERVAL};

/// Looks at the watched files every [`WATCH_INTERVAL`] updates and applies
/// what changed: the entry is replaced (or keeps its last good version and
/// shows why the file does not load), and the selected scenario restarts when
/// its file loaded anew.
pub(super) fn reload_changed_files(
    mut gallery: ResMut<Gallery>,
    mut state: ResMut<GalleryState>,
    mut updates: Local<u32>,
) {
    if !gallery.is_watching() || state.is_running_all() {
        return;
    }
    *updates = updates.wrapping_add(1);
    if !(*updates).is_multiple_of(WATCH_INTERVAL) {
        return;
    }
    let reloads = gallery.poll_files();
    if reloads.is_empty() {
        return;
    }
    for reload in reloads {
        match reload {
            Reload::Added(index) => {
                state.insert_entry(index);
                "A new scenario file was added".clone_into(&mut state.status);
            }
            Reload::Changed(index) => {
                let Some(entry) = gallery.get(index) else {
                    continue;
                };
                let name = entry.scenario.name().to_owned();
                if let Some(error) = entry.error() {
                    let first = error.lines().next().unwrap_or_default();
                    state.status = format!("{name} does not load: {first}");
                } else {
                    state.forget_results(index);
                    state.status = format!("Reloaded {name}");
                    if state.selected() == Some(index) && !state.is_recording() {
                        state.request(GalleryAction::Restart);
                    }
                }
            }
        }
    }
    state.revision = state.revision.wrapping_add(1);
}
