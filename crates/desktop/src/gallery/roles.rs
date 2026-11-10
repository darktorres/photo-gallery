//! The model's roles: a row's span, height and version, and the cells' lists, with their names in QML.

/// A row's items, (first, count) as a point: one value, so a row changed or reused is given its items at once.
pub(super) const ROLE_SPAN: i32 = 256;
pub(super) const ROLE_HEIGHT: i32 = 257;
/// A row's version: a new one, never given before, whenever the items the row holds change without its span
/// changing (rated, turned, marked, rewritten, a new list). Its cells read their items again on it, not on every
/// change anywhere (`revision`, which the viewer and the toolbar follow); and a delegate the view reuses for a row
/// knows from it whether its cells show that row as it is.
pub(super) const ROLE_VERSION: i32 = 258;
/// The values a row's cells show, one role per field: a list with an entry per item of the row, in order. They are
/// typed lists (`QStringList`, `QList<i32>`, `QList<bool>`, `QList<f64>`), which QML's compiler reads without `var`
/// (`GridRow`). Each comes with `ROLE_SPAN` and `ROLE_VERSION`, and before them (`show`, `items_changed`). A cell
/// needs no image key of its own: its thumbnail URLs carry it, and the viewer reads its item's (`viewedKey`).
pub(super) const ROLE_BADGES: i32 = 260;
pub(super) const ROLE_RATINGS: i32 = 261;
pub(super) const ROLE_BORDERS_MARKED: i32 = 262;
pub(super) const ROLE_IMAGE_ASPECTS: i32 = 263;
pub(super) const ROLE_ASPECTS: i32 = 264;
pub(super) const ROLE_CONTENT_X: i32 = 265;
pub(super) const ROLE_CONTENT_Y: i32 = 266;
pub(super) const ROLE_CONTENT_WIDTH: i32 = 267;
pub(super) const ROLE_CONTENT_HEIGHT: i32 = 268;
pub(super) const ROLE_THUMB_URLS: i32 = 269;
pub(super) const ROLE_THUMBHASH_URLS: i32 = 270;
/// The cells' roles, and their names in QML.
pub(super) const CELL_ROLES: [(i32, &str); 11] = [
    (ROLE_THUMB_URLS, "thumbUrls"),
    (ROLE_THUMBHASH_URLS, "thumbhashUrls"),
    (ROLE_BADGES, "badges"),
    (ROLE_RATINGS, "ratings"),
    (ROLE_BORDERS_MARKED, "bordersMarked"),
    (ROLE_IMAGE_ASPECTS, "imageAspects"),
    (ROLE_ASPECTS, "aspects"),
    (ROLE_CONTENT_X, "contentX"),
    (ROLE_CONTENT_Y, "contentY"),
    (ROLE_CONTENT_WIDTH, "contentWidth"),
    (ROLE_CONTENT_HEIGHT, "contentHeight"),
];

/// A count or a row as Qt's int, saturating: a Qt model holds no more rows than an int counts.
pub(super) fn qt_int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// The cells' roles (`CELL_ROLES`), for a change of their values.
pub(super) fn cell_roles() -> Vec<i32> {
    CELL_ROLES.iter().map(|&(role, _)| role).collect()
}
