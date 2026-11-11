//! The window's panels (S-12.20): whether the filter bar is open, whether the borders are shown, whether the viewer is
//! open (the grid's keys are the viewer's while it is), whether dev mode is on (the log view's button and key), and which
//! modal panels are open, in the order they opened. A change reports what changed, so the desktop shows it and
//! remembers the borders' setting; setting a flag to the value it has is no change. Nothing here calls Qt.
//!
//! The modal panels' open state is the QML's: each panel reports its opening and its closing (`PanelOpened`,
//! `PanelClosed`), and this only mirrors them, for the global keys' rule (`Controller`'s `Global`). The mirror is an
//! ordered set: a kind is in it at most once, so it holds at most [`PanelKind::ALL`]'s length by construction, whatever
//! the QML reports twice or never.
use serde::{Deserialize, Serialize};

/// A modal panel: one that takes the keys while it is open (a dialog, a menu, an overlay; not the viewer, whose keys are
/// its own by `SetViewerOpen`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelKind {
    Inbox,
    Log,
    Library,
    SaveViewDialog,
    ViewsMenu,
    FolderPicker,
    Yandex,
    YandexPreview,
}

impl PanelKind {
    /// Every kind, in the order of the enum `Kind` in PanelKind.qml (its codes are these positions).
    pub const ALL: [PanelKind; 8] = [
        PanelKind::Inbox,
        PanelKind::Log,
        PanelKind::Library,
        PanelKind::SaveViewDialog,
        PanelKind::ViewsMenu,
        PanelKind::FolderPicker,
        PanelKind::Yandex,
        PanelKind::YandexPreview,
    ];

    /// Whether the global keys switch between it and its sibling (the inbox and the log view); the other modals take
    /// every key while they are open.
    pub fn is_switchable(self) -> bool {
        matches!(self, PanelKind::Inbox | PanelKind::Log)
    }
}

/// A request about a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    ToggleFilters,
    SetFiltersOpen(bool),
    ToggleBorders,
    SetShowBorders(bool),
    SetViewerOpen(bool),
    /// Dev mode on or off, as the window starts with it (the preferences, `--dev=1`, `PG_DEV=1`): not saved.
    SetDevMode(bool),
    /// A modal panel opened. One of a kind already open moves to the top (it is the newest open), never in twice.
    PanelOpened(PanelKind),
    /// A modal panel closed: that kind leaves the panels open, wherever it is among them.
    PanelClosed(PanelKind),
}

/// A panel's flag that changed, with its new value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    FiltersOpen(bool),
    ShowBorders(bool),
    ViewerOpen(bool),
    DevMode(bool),
}

/// The panels' flags, as the core holds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Panels {
    filters_open: bool,
    show_borders: bool,
    viewer_open: bool,
    dev_mode: bool,
    /// The modal panels open, oldest first: an ordered set, each kind at most once.
    modals: Vec<PanelKind>,
}

impl Panels {
    /// Whether the filter bar is open.
    pub fn filters_open(&self) -> bool {
        self.filters_open
    }

    /// Whether the borders are shown.
    pub fn show_borders(&self) -> bool {
        self.show_borders
    }

    /// Whether the viewer is open.
    pub fn viewer_open(&self) -> bool {
        self.viewer_open
    }

    /// Whether dev mode is on: the log view can be opened.
    pub fn dev_mode(&self) -> bool {
        self.dev_mode
    }

    /// The modal panels open, oldest first, each kind once.
    pub fn modals(&self) -> &[PanelKind] {
        &self.modals
    }

    /// Whether a modal panel of this kind is open.
    pub fn is_open(&self, kind: PanelKind) -> bool {
        self.modals.contains(&kind)
    }

    /// Apply `event`; the changes it makes, in order (none when a flag already has the value).
    pub fn update(&mut self, event: Event) -> Vec<Change> {
        match event {
            Event::ToggleFilters => self.set_filters(!self.filters_open),
            Event::SetFiltersOpen(on) => self.set_filters(on),
            Event::ToggleBorders => self.set_borders(!self.show_borders),
            Event::SetShowBorders(on) => self.set_borders(on),
            Event::SetViewerOpen(on) => self.set_viewer(on),
            Event::SetDevMode(on) => self.set_dev_mode(on),
            Event::PanelOpened(kind) => {
                self.modals.retain(|k| *k != kind);
                self.modals.push(kind);
                Vec::new()
            }
            Event::PanelClosed(kind) => {
                self.modals.retain(|k| *k != kind);
                Vec::new()
            }
        }
    }

    fn set_viewer(&mut self, on: bool) -> Vec<Change> {
        if self.viewer_open == on {
            return Vec::new();
        }
        self.viewer_open = on;
        vec![Change::ViewerOpen(on)]
    }

    fn set_filters(&mut self, on: bool) -> Vec<Change> {
        if self.filters_open == on {
            return Vec::new();
        }
        self.filters_open = on;
        vec![Change::FiltersOpen(on)]
    }

    fn set_borders(&mut self, on: bool) -> Vec<Change> {
        if self.show_borders == on {
            return Vec::new();
        }
        self.show_borders = on;
        vec![Change::ShowBorders(on)]
    }

    fn set_dev_mode(&mut self, on: bool) -> Vec<Change> {
        if self.dev_mode == on {
            return Vec::new();
        }
        self.dev_mode = on;
        vec![Change::DevMode(on)]
    }
}
