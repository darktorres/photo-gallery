//! The cxx-qt bridge of [`crate::filters`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here.

use crate::bridges::filters_state::FiltersRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    /// A list facet of the filter bar, as QML names it (`Filters.Types`); maps to `pg_model::query::ListFacet` in
    /// `filters::list_facet`. Its values are in the bar's order (`pg_client::facets::ALL`).
    #[qenum(Filters)]
    enum ListFacetKind {
        Types,
        Shapes,
        Status,
        Borders,
        Ratings,
        Exts,
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// How many facets filter.
        #[qproperty(i32, active, READ, WRITE, NOTIFY, FINAL)]
        /// Part of the file name, any case; `""` doesn't filter.
        #[qproperty(QString, name, READ, WRITE, NOTIFY, FINAL)]
        /// The duplicates box: 0 either way; 1 files with a duplicate; 2 files without one.
        #[qproperty(i32, duplicates_index, cxx_name = "duplicatesIndex", READ, WRITE, NOTIFY, FINAL)]
        /// The review box: 0 either way; 1 borders not marked; 2 borders marked as not a border.
        #[qproperty(i32, border_marked_index, cxx_name = "borderMarkedIndex", READ, WRITE, NOTIFY, FINAL)]
        /// Files in the folder, unfiltered, and those the query keeps (`countFacets()`).
        #[qproperty(i32, total, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, matching, READ, WRITE, NOTIFY, FINAL)]
        /// The counts for the folder and query shown are not in yet: `total` and `matching` are the last ones read, or
        /// none (0). From the start and after a reload (`reload`), until `counted` delivers them: a pending state, not a
        /// zero.
        #[qproperty(bool, counts_pending, cxx_name = "countsPending", READ, WRITE, NOTIFY, FINAL)]
        /// One line saying what the query keeps, for the save-view dialog.
        #[qproperty(QString, description, READ, WRITE, NOTIFY, FINAL)]
        type Filters = super::FiltersRust;
    }

    impl cxx_qt::Threading for Filters {}

    unsafe extern "RustQt" {
        /// The counts `countFacets` asked for are in (and the choices show them).
        #[qsignal]
        fn counted(self: Pin<&mut Filters>);

        /// The user typed a name: the query takes it (`name` then shows it).
        #[qinvokable]
        #[cxx_name = "editName"]
        fn edit_name(self: Pin<&mut Filters>, value: &QString);

        /// The user chose a box's index (`duplicatesIndex`, `borderMarkedIndex` then show it).
        #[qinvokable]
        #[cxx_name = "chooseDuplicates"]
        fn choose_duplicates(self: Pin<&mut Filters>, index: i32);

        #[qinvokable]
        #[cxx_name = "chooseBorderMarked"]
        fn choose_border_marked(self: Pin<&mut Filters>, index: i32);

        /// The user changed the query.
        #[qsignal]
        fn edited(self: Pin<&mut Filters>);

        /// The query was replaced (a saved view, Clear): fields showing typed text reload it.
        #[qsignal]
        fn loaded(self: Pin<&mut Filters>);

        /// Show the listing's query again: a saved view was applied, so the fields show its query (`loaded`).
        #[qinvokable]
        fn reload(self: Pin<&mut Filters>);

        /// Choose a value of a list facet, or unchoose it. `facet` is the `Filters.ListFacetKind` value as an int (QML's
        /// type checker cannot resolve the enum as a method parameter; `filters::kind_of` maps it back).
        #[qinvokable]
        fn toggle(self: Pin<&mut Filters>, facet: i32, value: &QString);

        /// One end of a bound as text, `""` when open: `field` is the bound's field's index in `input::Field::ALL`
        /// (the range facets' numbers and the day facets' days), `end` its `input::Edge` index (0 low, 1 high).
        #[qinvokable]
        fn bound(self: &Filters, field: i32, end: i32) -> QString;

        /// Set one end of a bound from what was typed; text that doesn't parse leaves it open.
        #[qinvokable]
        #[cxx_name = "setBound"]
        fn set_bound(self: Pin<&mut Filters>, field: i32, end: i32, text: &QString);

        /// Whether text typed for a bound's field is empty or parses (`input::bound_is_valid`).
        #[qinvokable]
        #[cxx_name = "boundValid"]
        fn bound_valid(self: &Filters, field: i32, text: &QString) -> bool;

        /// Filter nothing.
        #[qinvokable]
        fn clear(self: Pin<&mut Filters>);

        /// Count the files of the listed folder (the controller's listing: its folder, with or without its subfolders):
        /// `total`, `matching`, the types', extensions' and borders' counts. Counted off the GUI thread; they change when
        /// the counts are in (`counted`).
        #[qinvokable]
        #[cxx_name = "countFacets"]
        fn count_facets(self: Pin<&mut Filters>);

        /// The filter bar opened or closed. Opening it counts the choices.
        #[qinvokable]
        #[cxx_name = "setBarOpen"]
        fn set_bar_open(self: Pin<&mut Filters>, open: bool);

        /// Count the choices when they are shown: the bar is open or a filter is set.
        #[qinvokable]
        #[cxx_name = "countIfNeeded"]
        fn count_if_needed(self: Pin<&mut Filters>);
    }
}
