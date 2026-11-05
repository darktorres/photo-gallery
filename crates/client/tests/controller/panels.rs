//! Traces of the panels' flags (S-12.20).

use pg_client::controller::panels::{Change, Event, Panels};

#[test]
fn toggles_and_sets_report_only_real_changes() {
    let mut panels = Panels::default();
    assert_eq!(panels.update(Event::ToggleFilters), vec![Change::FiltersOpen(true)]);
    assert_eq!(panels.update(Event::SetFiltersOpen(true)), vec![]);
    assert_eq!(panels.update(Event::ToggleFilters), vec![Change::FiltersOpen(false)]);
    assert_eq!(panels.update(Event::ToggleBorders), vec![Change::ShowBorders(true)]);
    assert_eq!(panels.update(Event::SetShowBorders(true)), vec![]);
    assert_eq!(panels.update(Event::SetShowBorders(false)), vec![Change::ShowBorders(false)]);
    assert!(!panels.filters_open());
    assert!(!panels.show_borders());
}

#[test]
fn the_viewer_flag_reports_only_real_changes_and_leaves_the_others() {
    let mut panels = Panels::default();
    assert!(!panels.viewer_open());
    assert_eq!(panels.update(Event::SetViewerOpen(true)), vec![Change::ViewerOpen(true)]);
    assert_eq!(panels.update(Event::SetViewerOpen(true)), vec![]);
    assert!(panels.viewer_open());
    // Opening the filter bar while the viewer is open is the bar's own change, and the viewer's flag stays.
    assert_eq!(panels.update(Event::ToggleFilters), vec![Change::FiltersOpen(true)]);
    assert!(panels.viewer_open());
    assert_eq!(panels.update(Event::SetViewerOpen(false)), vec![Change::ViewerOpen(false)]);
    assert_eq!(panels.update(Event::SetViewerOpen(false)), vec![]);
    assert!(!panels.viewer_open());
    assert!(panels.filters_open());
    assert!(!panels.show_borders());
}
