//! The window's panels (S-12.20): whether the filter bar is open, whether the borders are shown, and whether the viewer is
//! open (the grid's keys are the viewer's while it is). A change reports what changed, so the desktop shows it and
//! remembers the borders' setting; setting a flag to the value it has is no change. Nothing here calls Qt.

/// A request about a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    ToggleFilters,
    SetFiltersOpen(bool),
    ToggleBorders,
    SetShowBorders(bool),
    SetViewerOpen(bool),
}

/// A panel's flag that changed, with its new value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    FiltersOpen(bool),
    ShowBorders(bool),
    ViewerOpen(bool),
}

/// The panels' flags, as the core holds them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Panels {
    filters_open: bool,
    show_borders: bool,
    viewer_open: bool,
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

    /// Apply `event`; the changes it makes, in order (none when a flag already has the value).
    pub fn update(&mut self, event: Event) -> Vec<Change> {
        match event {
            Event::ToggleFilters => self.set_filters(!self.filters_open),
            Event::SetFiltersOpen(on) => self.set_filters(on),
            Event::ToggleBorders => self.set_borders(!self.show_borders),
            Event::SetShowBorders(on) => self.set_borders(on),
            Event::SetViewerOpen(on) => self.set_viewer(on),
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
}
