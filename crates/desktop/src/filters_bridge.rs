//! The cxx-qt bridge of [`crate::filters`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::filters::state::FiltersRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// How many facets filter.
        #[qproperty(i32, active, READ, WRITE, NOTIFY, FINAL)]
        /// Part of the file name, any case; `""` doesn't filter.
        #[qproperty(QString, name, READ, WRITE = set_name_edited, NOTIFY, FINAL)]
        /// The duplicates box: 0 either way; 1 files with a duplicate; 2 files without one.
        #[qproperty(i32, duplicates_index, cxx_name = "duplicatesIndex", READ, NOTIFY, FINAL)]
        /// The review box: 0 either way; 1 borders not marked; 2 borders marked as not a border.
        #[qproperty(i32, border_marked_index, cxx_name = "borderMarkedIndex", READ, NOTIFY, FINAL)]
        /// Files in the folder, unfiltered, and those the query keeps (`countFacets()`).
        #[qproperty(i32, total, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, matching, READ, WRITE, NOTIFY, FINAL)]
        /// One line saying what the query keeps, for the save-view dialog.
        #[qproperty(QString, description, READ, WRITE, NOTIFY, FINAL)]
        type Filters = super::FiltersRust;
    }

    impl cxx_qt::Threading for Filters {}

    unsafe extern "RustQt" {
        /// The counts `countFacets` asked for are in (and the choices show them).
        #[qsignal]
        fn counted(self: Pin<&mut Filters>);

        fn set_name_edited(self: Pin<&mut Filters>, value: &QString);

        /// Choose a box's index (`duplicatesIndex`, `borderMarkedIndex`).
        #[qinvokable]
        #[cxx_name = "setDuplicatesIndex"]
        fn set_duplicates_index(self: Pin<&mut Filters>, index: i32);

        #[qinvokable]
        #[cxx_name = "setBorderMarkedIndex"]
        fn set_border_marked_index(self: Pin<&mut Filters>, index: i32);

        /// The user changed the query.
        #[qsignal]
        fn edited(self: Pin<&mut Filters>);

        /// The query was replaced (a saved view, Clear): fields showing typed text reload it.
        #[qsignal]
        fn loaded(self: Pin<&mut Filters>);

        /// Show the listing's query again: a saved view was applied, so the fields show its query (`loaded`).
        #[qinvokable]
        fn reload(self: Pin<&mut Filters>);

        /// Choose a value of a list facet (its index in `pg_client::facets::Facet`), or unchoose it.
        #[qinvokable]
        fn toggle(self: Pin<&mut Filters>, facet: i32, value: &QString);

        /// One end of a bound as text, `""` when open: `facet` is the bound's field's index in `input::Field::ALL`
        /// (the range facets' numbers and the day facets' days), `end` its `input::Edge` index (0 low, 1 high).
        #[qinvokable]
        fn bound(self: &Filters, facet: i32, end: i32) -> QString;

        /// Set one end of a bound from what was typed; text that doesn't parse leaves it open.
        #[qinvokable]
        #[cxx_name = "setBound"]
        fn set_bound(self: Pin<&mut Filters>, facet: i32, end: i32, text: &QString);

        /// Whether text typed for a bound's field is empty or parses (`input::bound_is_valid`).
        #[qinvokable]
        #[cxx_name = "boundValid"]
        fn bound_valid(self: &Filters, facet: i32, text: &QString) -> bool;

        /// Filter nothing.
        #[qinvokable]
        fn clear(self: Pin<&mut Filters>);

        /// Count the files of the listed folder (the controller's listing: its folder, with or without its subfolders):
        /// `total`, `matching`, the types', extensions' and borders' counts. Counted off the GUI thread; they change when
        /// the counts are in (`counted`).
        #[qinvokable]
        #[cxx_name = "countFacets"]
        fn count_facets(self: Pin<&mut Filters>);
    }
}
