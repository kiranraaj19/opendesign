//! Task modes: dock layouts arranged for one kind of work, picked from
//! View -> Workspace (View -> Reset layout gives the everyday one).
//! Every layout keeps every tab, so a saved layout made from one is
//! restored like any other (see `app::restore_layout`).

use egui_dock::{DockState, NodeIndex};

use crate::app::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskMode {
    /// Drafting: a wide 2D window, the 3D view small beside it.
    Pattern,
    /// Draping: a wide 3D view, the 2D window small beside it.
    Simulation,
}

impl TaskMode {
    pub const ALL: [TaskMode; 2] = [TaskMode::Pattern, TaskMode::Simulation];

    pub fn label(self) -> &'static str {
        match self {
            TaskMode::Pattern => "Pattern",
            TaskMode::Simulation => "Simulation",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            TaskMode::Pattern => "A wide 2D window for drafting, the 3D view small",
            TaskMode::Simulation => "A wide 3D view for draping, the 2D window small",
        }
    }

    /// Library and Objects on the left, Properties and History on the
    /// right, and the two views in the middle, split as the mode wants.
    pub fn layout(self) -> DockState<Tab> {
        // `split_right` sizes the left part with the fraction.
        let (first, second) = match self {
            TaskMode::Pattern => (Tab::Pattern2D, Tab::Viewport3D),
            TaskMode::Simulation => (Tab::Viewport3D, Tab::Pattern2D),
        };
        let mut dock = DockState::new(vec![first]);
        let tree = dock.main_surface_mut();
        let [center, _] =
            tree.split_right(NodeIndex::root(), 0.8, vec![Tab::Properties, Tab::History]);
        let [center, _] = tree.split_left(center, 0.2, vec![Tab::Library, Tab::Objects]);
        tree.split_right(center, 0.75, vec![second]);
        dock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_keeps_every_tab() {
        for mode in TaskMode::ALL {
            let dock = mode.layout();
            for tab in Tab::ALL {
                assert!(dock.find_tab(&tab).is_some(), "{mode:?} lacks {tab:?}");
            }
        }
    }
}
