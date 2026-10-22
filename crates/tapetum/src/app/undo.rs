//! Undo/redo (Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z, Ctrl+Y) over everything edited
//! in the visualizer: the project (types, objects, scenes, render settings)
//! including moved fixture placements. A drag or a burst of typing is one step.
//! Undoing a placement or a range sends it to BlinderKitten again.

use crate::i18n::tr;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bk_olga::Attribute;
use tapetum_project::Project;

use super::layout;
use super::state::{PlacementSync, RangeSync, ShowState};

/// Steps kept.
const DEPTH: usize = 200;
/// A change becomes a step once nothing changed for this long (and no mouse button is held).
const SETTLE: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Undo,
    Redo,
}

#[derive(Resource, Default)]
pub struct UndoHistory {
    undo: Vec<Project>,
    redo: Vec<Project>,
    /// The project as of the last step.
    committed: Option<Project>,
    /// Latest uncommitted state seen, and when it last changed.
    pending: Option<(Project, Instant)>,
    /// Asked for by the keyboard or the buttons; done in [`track_undo`].
    pub request: Option<Step>,
}

impl UndoHistory {
    pub fn can_undo(&self, current: &Project) -> bool {
        !self.undo.is_empty() || self.committed.as_ref().is_some_and(|c| c != current)
    }

    pub fn can_redo(&self, current: &Project) -> bool {
        !self.redo.is_empty() && self.committed.as_ref() == Some(current)
    }

    /// The project changed by itself (BK saved, the show reloaded): no step.
    pub fn external(&mut self, before: &Project, after: &Project) {
        if self.committed.as_ref() == Some(before) {
            self.committed = Some(after.clone());
        }
    }

    fn commit(&mut self, current: &Project) {
        if let Some(prev) = self.committed.replace(current.clone())
            && prev != *current
        {
            self.undo.push(prev);
            if self.undo.len() > DEPTH {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.pending = None;
    }
}

pub fn track_undo(
    mut history: ResMut<UndoHistory>,
    mut state: ResMut<ShowState>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut placements: ResMut<PlacementSync>,
    mut ranges: ResMut<RangeSync>,
) {
    let Some(committed) = history.committed.as_ref() else {
        history.committed = Some(state.project.clone());
        return;
    };

    if *committed != state.project {
        let still_changing = history.pending.as_ref().is_none_or(|(p, _)| *p != state.project);
        if still_changing {
            history.pending = Some((state.project.clone(), Instant::now()));
        }
        let settled = history.pending.as_ref().is_some_and(|(_, t)| t.elapsed() >= SETTLE);
        if (settled && buttons.get_pressed().next().is_none()) || history.request.is_some() {
            let current = state.project.clone();
            history.commit(&current);
        }
    } else {
        history.pending = None;
    }

    let Some(step) = history.request.take() else { return };
    let current = state.project.clone();
    let target = match step {
        Step::Undo => history.undo.pop().inspect(|_| history.redo.push(current.clone())),
        Step::Redo => history.redo.pop().inspect(|_| history.undo.push(current.clone())),
    };
    let Some(mut target) = target else { return };
    resend(&state, &current, &mut target, &mut placements, &mut ranges);
    let rebuild = target.render.quality != current.render.quality || target.fixtures != current.fixtures || target.fixture_types != current.fixture_types;
    history.committed = Some(target.clone());
    history.pending = None;
    state.project = target;
    if rebuild {
        state.generation += 1;
    }
    state.save_project();
    state.status = Some(match step {
        Step::Undo => tr("Rückgängig gemacht", "Undone").into(),
        Step::Redo => tr("Wiederhergestellt", "Redone").into(),
    });
}

/// Placements and ranges that differ after the step go to BK again. A fixture
/// whose step returns it to where BK has it gets that placement explicitly, so
/// BK is moved back even before it saves.
fn resend(state: &ShowState, before: &Project, after: &mut Project, placements: &mut PlacementSync, ranges: &mut RangeSync) {
    let old = layout::placements(&state.show, before);
    let new = layout::placements(&state.show, after);
    for ((f, a), b) in state.show.fixtures.iter().zip(&old).zip(&new) {
        if a.position != b.position || a.rotation != b.rotation {
            after.pending_placements.insert(f.id, *b);
            placements.queue(f.id, &f.short_name, *b);
        }
    }
    for ty in &state.show.fixture_types {
        let (a, b) = (before.fixture_types.get(&ty.short_name), after.fixture_types.get(&ty.short_name));
        for attr in [Attribute::Pan, Attribute::Tilt, Attribute::Zoom] {
            let field = |o: Option<&tapetum_project::FixtureOverride>| {
                o.and_then(|o| match attr {
                    Attribute::Pan => o.pan_range,
                    Attribute::Tilt => o.tilt_range,
                    _ => o.zoom_range,
                })
            };
            if field(a) != field(b)
                && let Some(range) = field(b).or_else(|| super::fixtures::bk_range(ty, &attr))
            {
                ranges.queue(&ty.short_name, attr.clone(), range);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(ambient: f32) -> Project {
        let mut p = Project::default();
        p.render.ambient = ambient;
        p
    }

    #[test]
    fn steps_go_back_and_forth() {
        let mut h = UndoHistory { committed: Some(project(0.1)), ..default() };
        h.commit(&project(0.2));
        h.commit(&project(0.3));
        assert!(h.can_undo(&project(0.3)));
        assert_eq!(h.undo.pop().unwrap(), project(0.2));
        // A new change after undoing drops the redo steps.
        h.redo.push(project(0.3));
        h.committed = Some(project(0.2));
        h.commit(&project(0.5));
        assert!(h.redo.is_empty());
        assert_eq!(h.undo.last().unwrap(), &project(0.2));
    }

    #[test]
    fn external_changes_are_no_step() {
        let mut h = UndoHistory { committed: Some(project(0.1)), ..default() };
        h.external(&project(0.1), &project(0.4));
        assert!(!h.can_undo(&project(0.4)));
    }
}
