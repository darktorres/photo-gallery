//! A star rating, 1 to 5. Unrated is `None`, everywhere a rating can be absent: the catalog's `ratings` and
//! `file_ratings` (where unrated is no row), the catalog's and a file's rating in memory, and the reader's result.
//! Zero is never a rating. It is only the spelling of unrated at the edges: a file's EXIF `Rating` or XMP `xmp:Rating`
//! of 0, the job's wire value, and the 0 key. Each of those edges turns it into `None` (or back) in one place.

/// Stars, 1 to 5. Serialized as its number of stars; a number outside 1 to 5 doesn't read.
#[cfg_attr(feature = "specta", derive(specta::Type), specta(type = u8))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Rating(u8);

/// Why a star count is refused as a rating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RatingError {
    /// Zero stars: that is unrated (`None`), not a rating.
    Unrated,
    /// The count is past [`Rating::MAX`].
    TooManyStars(u8),
}

impl core::fmt::Display for RatingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unrated => write!(f, "0 stars is unrated, not a rating"),
            Self::TooManyStars(stars) => write!(f, "{stars} stars: a rating has at most {}", Rating::MAX),
        }
    }
}

impl core::error::Error for RatingError {}

impl TryFrom<u8> for Rating {
    type Error = RatingError;

    fn try_from(stars: u8) -> Result<Rating, RatingError> {
        match stars {
            0 => Err(RatingError::Unrated),
            _ => Rating::new(stars).ok_or(RatingError::TooManyStars(stars)),
        }
    }
}

impl From<Rating> for u8 {
    fn from(rating: Rating) -> u8 {
        rating.0
    }
}

impl Rating {
    /// The most stars a rating has.
    pub const MAX: u8 = 5;

    /// `stars` stars: `None` for 0 (unrated) and for a count above [`Rating::MAX`].
    pub const fn new(stars: u8) -> Option<Rating> {
        if stars >= 1 && stars <= Rating::MAX { Some(Rating(stars)) } else { None }
    }

    /// The number of stars, 1 to 5.
    pub const fn get(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_unrated_not_a_rating() {
        assert_eq!(Rating::new(0), None);
        assert_eq!(Rating::try_from(0), Err(RatingError::Unrated));
        assert_eq!(Rating::new(1).map(Rating::get), Some(1));
        assert_eq!(Rating::new(5).map(Rating::get), Some(5));
        assert_eq!(Rating::new(6), None);
        assert_eq!(Rating::try_from(6), Err(RatingError::TooManyStars(6)));
    }
}
