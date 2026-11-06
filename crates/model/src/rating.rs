//! A star rating: what the file itself says (`file_ratings`, 0 to 5 stars), and the rating the user gave (`ratings`,
//! 1 to 5). An unrated file has no rating at all, so it is `Option<Rating>`.

/// Stars, 0 to 5.
#[cfg_attr(feature = "specta", derive(specta::Type), specta(type = u8))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Rating(u8);

impl Rating {
    /// The most stars a rating has.
    pub const MAX: u8 = 5;

    /// `stars` stars; `None` above [`Rating::MAX`].
    pub const fn new(stars: u8) -> Option<Rating> {
        if stars <= Rating::MAX { Some(Rating(stars)) } else { None }
    }

    /// The number of stars, 0 to 5.
    pub const fn get(self) -> u8 {
        self.0
    }
}
