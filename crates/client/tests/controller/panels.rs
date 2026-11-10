//! Traces of the panels' flags (S-12.20), the modal panels' mirror and the global shortcuts' rule.

use pg_client::controller::panels::{Change, Event, PanelKind, Panels};
use pg_client::controller::{Command, Controller, Event as Report};
use pg_client::keys::Global;
use pg_model::query::{Query, Sort};
use pg_model::rules::Merged;
use pg_model::view::Listing;

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

/// A dialog opened from the library panel closes first: each close names its kind, so it takes off the right one.
#[test]
fn nested_modals_close_by_their_kind() {
    let mut panels = Panels::default();
    assert_eq!(panels.update(Event::PanelOpened(PanelKind::Library)), vec![]);
    panels.update(Event::PanelOpened(PanelKind::FolderPicker));
    assert_eq!(panels.modals(), [PanelKind::Library, PanelKind::FolderPicker]);
    panels.update(Event::PanelClosed(PanelKind::Library));
    assert_eq!(panels.modals(), [PanelKind::FolderPicker]);
    panels.update(Event::PanelClosed(PanelKind::FolderPicker));
    assert!(panels.modals().is_empty());
    // A close of a kind that isn't open changes nothing.
    panels.update(Event::PanelClosed(PanelKind::Log));
    assert!(panels.modals().is_empty());
}

#[test]
fn dev_mode_as_the_window_starts_reports_only_a_real_change() {
    let mut panels = Panels::default();
    assert_eq!(panels.update(Event::SetDevMode(false)), vec![]);
    assert_eq!(panels.update(Event::SetDevMode(true)), vec![Change::DevMode(true)]);
    assert!(panels.dev_mode());
}

fn controller() -> Controller {
    let listing =
        Listing { folder: Merged::default(), recursive: true, sort: Sort::default(), query: Query::default() };
    Controller::new(listing, 200, 1.0)
}

fn report(c: &mut Controller, event: Report) -> Vec<Command> {
    c.update(0, event)
}

#[test]
fn the_log_key_does_nothing_out_of_dev_mode() {
    let mut c = controller();
    assert_eq!(report(&mut c, Report::Global(Global::Log)), vec![]);
}

/// Dev mode's key turns it on and saves that; a start's dev mode (`--dev=1`) is not saved.
#[test]
fn the_dev_mode_key_saves_it_and_the_start_does_not() {
    let mut c = controller();
    assert_eq!(
        report(&mut c, Report::Panel(Event::SetDevMode(true))),
        vec![Command::Panel(Change::DevMode(true))],
        "the start's dev mode is shown, not saved"
    );
    assert_eq!(
        report(&mut c, Report::Global(Global::DevMode)),
        vec![Command::Panel(Change::DevMode(false)), Command::SaveDevMode(false)]
    );
    assert_eq!(
        report(&mut c, Report::Global(Global::DevMode)),
        vec![Command::Panel(Change::DevMode(true)), Command::SaveDevMode(true)]
    );
}

/// In dev mode the log's key opens it, and closes it when it is open (the panel reports its own opening).
#[test]
fn the_log_key_toggles_the_log() {
    let mut c = controller();
    report(&mut c, Report::Panel(Event::SetDevMode(true)));
    assert_eq!(report(&mut c, Report::Global(Global::Log)), vec![Command::OpenPanel(PanelKind::Log)]);
    report(&mut c, Report::Panel(Event::PanelOpened(PanelKind::Log)));
    assert_eq!(report(&mut c, Report::Global(Global::Log)), vec![Command::ClosePanel(PanelKind::Log)]);
}

/// Turning dev mode off closes the log, which needs it.
#[test]
fn turning_dev_mode_off_closes_the_log() {
    let mut c = controller();
    report(&mut c, Report::Panel(Event::SetDevMode(true)));
    report(&mut c, Report::Panel(Event::PanelOpened(PanelKind::Log)));
    assert_eq!(
        report(&mut c, Report::Global(Global::DevMode)),
        vec![Command::Panel(Change::DevMode(false)), Command::SaveDevMode(false), Command::ClosePanel(PanelKind::Log)]
    );
}

/// From the inbox, the log's key closes the inbox and opens the log.
#[test]
fn the_log_key_switches_from_the_inbox() {
    let mut c = controller();
    report(&mut c, Report::Panel(Event::SetDevMode(true)));
    report(&mut c, Report::Panel(Event::PanelOpened(PanelKind::Inbox)));
    assert_eq!(
        report(&mut c, Report::Global(Global::Log)),
        vec![Command::ClosePanel(PanelKind::Inbox), Command::OpenPanel(PanelKind::Log)]
    );
}

/// While a dialog or a menu is open, it takes every key: the global shortcuts do nothing, dev mode's included.
#[test]
fn another_modal_takes_the_global_keys() {
    let mut c = controller();
    report(&mut c, Report::Panel(Event::SetDevMode(true)));
    for kind in PanelKind::ALL.into_iter().filter(|k| !k.is_switchable()) {
        report(&mut c, Report::Panel(Event::PanelOpened(kind)));
        assert_eq!(report(&mut c, Report::Global(Global::Log)), vec![], "{kind:?}");
        assert_eq!(report(&mut c, Report::Global(Global::DevMode)), vec![], "{kind:?}");
        report(&mut c, Report::Panel(Event::PanelClosed(kind)));
    }
    assert!(c.panels().dev_mode());
}

#[test]
fn the_new_events_and_commands_read_back_as_written() {
    for event in [
        Report::Global(Global::Log),
        Report::Panel(Event::PanelOpened(PanelKind::YandexPreview)),
        Report::Panel(Event::PanelClosed(PanelKind::Inbox)),
        Report::Panel(Event::SetDevMode(true)),
    ] {
        let text = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Report>(&text).unwrap(), event);
    }
    for command in [Command::OpenPanel(PanelKind::Log), Command::ClosePanel(PanelKind::Log), Command::SaveDevMode(true)]
    {
        let text = serde_json::to_string(&command).unwrap();
        assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
    }
}
