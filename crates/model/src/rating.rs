//! A star rating: what the file itself says (`file_ratings`, 0 to 5 stars), and the rating the user gave (`ratings`,
//! 1 to 5 stars; a user's 0 stars clears the row, so it is no rating). An unrated file has no rating at all, so it is
//! `Option<Rating>`.

/// Stars, 0 to 5. Serialized as its number of stars; a number past 5 doesn't read.
#[cfg_attr(feature = "specta", derive(specta::Type), specta(type = u8))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Rating(u8);

/// Why a star count is refused as a rating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RatingError {
    /// The count is past [`Rating::MAX`].
    TooManyStars(u8),
}

impl std::fmt::Display for RatingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyStars(stars) => write!(f, "{stars} stars: a rating has at most {}", Rating::MAX),
        }
    }
}

impl std::error::Error for RatingError {}

impl TryFrom<u8> for Rating {
    type Error = RatingError;

    fn try_from(stars: u8) -> Result<Rating, RatingError> {
        Rating::new(stars).ok_or(RatingError::TooManyStars(stars))
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

    /// `stars` stars; `None` above [`Rating::MAX`].
    pub const fn new(stars: u8) -> Option<Rating> {
        if stars <= Rating::MAX { Some(Rating(stars)) } else { None }
    }

    /// The number of stars, 0 to 5.
    pub const fn get(self) -> u8 {
        self.0
    }
}
