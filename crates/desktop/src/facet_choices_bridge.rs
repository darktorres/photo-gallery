//! The cxx-qt bridge of [`crate::facet_choices`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::facet_choices::state::FacetChoicesRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        /// Which list, by its index in `pg_client::facets::Facet` (the filter bar names them). Set it once.
        #[qproperty(i32, facet, READ, WRITE = set_facet, FINAL)]
        type FacetChoices = super::FacetChoicesRust;
    }

    unsafe extern "RustQt" {
        fn set_facet(self: Pin<&mut FacetChoices>, value: i32);

        /// Rebuild the rows from the listing's query and the counts (after a change to either).
        #[qinvokable]
        fn refresh(self: Pin<&mut FacetChoices>);

        #[qinvokable]
        #[cxx_override]
        fn data(self: &FacetChoices, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &FacetChoices) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &FacetChoices, parent: &QModelIndex) -> i32;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut FacetChoices>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut FacetChoices>);
    }
}
