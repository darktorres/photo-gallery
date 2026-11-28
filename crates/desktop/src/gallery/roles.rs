//! The model's roles: a row's values, one role per field, and its height, with their names in QML.

/// The row's first item (the first of its items, in the list's order).
pub(super) const ROLE_FIRST: i32 = 256;
/// The row's number of items.
pub(super) const ROLE_COUNT: i32 = 257;
/// The file each item is (`ids`): a cell whose file changes makes a new cell, so this is what the QML compares.
pub(super) const ROLE_IDS: i32 = 258;
/// The thumbnail URL of each item (at the thumbnail height).
pub(super) const ROLE_THUMB_URLS: i32 = 259;
/// The ThumbHash placeholder URL of each item.
pub(super) const ROLE_THUMBHASH_URLS: i32 = 260;
/// The badge of each item (an animated image's, or empty).
pub(super) const ROLE_BADGES: i32 = 261;
/// The rating of each item.
pub(super) const ROLE_RATINGS: i32 = 262;
/// Whether each item's borders are marked as not borders.
pub(super) const ROLE_BORDERS_MARKED: i32 = 263;
/// The aspect of each item's image as shown.
pub(super) const ROLE_IMAGE_ASPECTS: i32 = 264;
/// The aspect of each item's tile.
pub(super) const ROLE_ASPECTS: i32 = 265;
/// Each item's borders' box, left as fractions of the image: x, y, width and height (one role each).
pub(super) const ROLE_CONTENT_X: i32 = 266;
pub(super) const ROLE_CONTENT_Y: i32 = 267;
pub(super) const ROLE_CONTENT_WIDTH: i32 = 268;
pub(super) const ROLE_CONTENT_HEIGHT: i32 = 269;
/// A row's height, made from the thumbnail height. Signalled alone when only the height changed.
pub(super) const ROLE_HEIGHT: i32 = 270;
/// Whether each item is an animated image (its cell plays it, on screen).
pub(super) const ROLE_ANIMATED: i32 = 271;
/// The quarter turns and the mirror each item's animation is shown with (`player_turns`, the viewer's rule).
pub(super) const ROLE_PLAYER_TURNS: i32 = 272;
pub(super) const ROLE_PLAYER_MIRRORED: i32 = 273;

/// The roles that name a row's values. The count is last: a row's change names its values in this order, so the count
/// is the last of them to land (what GridRow's Repeater reads first is then already the new values).
const VALUE_ROLES: [i32; 17] = [
    ROLE_IDS,
    ROLE_THUMB_URLS,
    ROLE_THUMBHASH_URLS,
    ROLE_BADGES,
    ROLE_RATINGS,
    ROLE_BORDERS_MARKED,
    ROLE_IMAGE_ASPECTS,
    ROLE_ASPECTS,
    ROLE_CONTENT_X,
    ROLE_CONTENT_Y,
    ROLE_CONTENT_WIDTH,
    ROLE_CONTENT_HEIGHT,
    ROLE_ANIMATED,
    ROLE_PLAYER_TURNS,
    ROLE_PLAYER_MIRRORED,
    ROLE_FIRST,
    ROLE_COUNT,
];

/// Every role with its name in QML (the model's role names).
pub(super) const ROLE_NAMES: [(i32, &str); 18] = [
    (ROLE_FIRST, "first"),
    (ROLE_COUNT, "count"),
    (ROLE_IDS, "ids"),
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
    (ROLE_ANIMATED, "animated"),
    (ROLE_PLAYER_TURNS, "playerTurns"),
    (ROLE_PLAYER_MIRRORED, "playerMirrored"),
    (ROLE_HEIGHT, "rowHeight"),
];

/// The roles one row's change names, in the order they are sent: its values (the count last) when `values` (its span or
/// its items changed), then its height when `height` changed.
pub(super) fn row_change_roles(values: bool, height: bool) -> Vec<i32> {
    let mut roles = Vec::with_capacity(VALUE_ROLES.len() + 1);
    if values {
        roles.extend_from_slice(&VALUE_ROLES);
    }
    if height {
        roles.push(ROLE_HEIGHT);
    }
    roles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_change_names_every_field_with_the_count_last() {
        let roles = row_change_roles(true, false);
        assert_eq!(roles.len(), VALUE_ROLES.len());
        assert_eq!(roles.last(), Some(&ROLE_COUNT), "the count is the last role named");
        assert_eq!(roles[roles.len() - 2], ROLE_FIRST, "the first item is named just before it");
        assert!(!roles.contains(&ROLE_HEIGHT));
    }

    #[test]
    fn a_height_change_alone_names_only_the_height() {
        assert_eq!(row_change_roles(false, true), vec![ROLE_HEIGHT]);
    }

    #[test]
    fn a_span_change_with_a_height_change_names_the_values_then_the_height() {
        let roles = row_change_roles(true, true);
        assert_eq!(roles.last(), Some(&ROLE_HEIGHT));
        assert_eq!(roles.len(), VALUE_ROLES.len() + 1);
    }

    #[test]
    fn no_change_names_no_role() {
        assert!(row_change_roles(false, false).is_empty());
    }

    #[test]
    fn every_value_role_has_a_name_and_no_two_roles_share_one() {
        let mut names: Vec<&str> = ROLE_NAMES.iter().map(|(_, name)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ROLE_NAMES.len(), "role names are unique");
        for role in VALUE_ROLES {
            assert!(ROLE_NAMES.iter().any(|(r, _)| *r == role), "value role {role} is named");
        }
    }
}
