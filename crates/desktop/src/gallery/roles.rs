//! The model's roles: a row as one value, and its height, with their names in QML.

/// A row, as one value (a map, `QVariantMap`): its items (`first`, `count`), its `version`, and the values its cells
/// show, one list per field with an entry per item of the row, in order (`thumbUrls`, `thumbhashUrls`, `badges`,
/// `ratings`, `bordersMarked`, `imageAspects`, `aspects`, `contentX`, `contentY`, `contentWidth`, `contentHeight`).
/// The model gives a row in one change, so `GridRow` takes it whole: no value is in before another. A row's version is
/// new, never given before, whenever its items change without its span changing (rated, turned, marked, rewritten, a
/// new list), and a delegate the view reuses for a row takes the row as it is. A cell needs no image key of its own:
/// its thumbnail URLs carry it, and the viewer reads its item's (`viewedKey`).
pub(super) const ROLE_ROW: i32 = 256;
/// A row's height, made from the thumbnail height. Signalled alone when only the height changed.
pub(super) const ROLE_HEIGHT: i32 = 257;

/// A count or a row as Qt's int, saturating: a Qt model holds no more rows than an int counts.
pub(super) fn qt_int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}
