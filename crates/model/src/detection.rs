//! What border detection found, as data: the sides and the content box, the reasons a candidate was rejected, and the
//! limits they live under. The detector (core's `borders` module) makes them; the catalog keeps them, and the grid
//! and the crop read them. No pixels and no I/O here.

/// [`MIN_SIDE`] as a literal, for SQL made at compile time (`catalog`'s `has_borders!`).
#[macro_export]
macro_rules! min_side {
    () => {
        64
    };
}
/// Images with a side shorter than this aren't looked at.
pub const MIN_SIDE: usize = min_side!();

#[derive(Clone, Copy, PartialEq, Eq, Debug, strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    /// The name in the catalog (`border_sides.edge`).
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<Edge> {
        key.parse().ok()
    }

    pub fn opposite(self) -> Edge {
        match self {
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum Kind {
    Solid,
    Blur,
}

impl Kind {
    /// The name in the catalog (`border_sides.kind`).
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        key.parse().ok()
    }
}

/// What a side looks like, for the filters: a solid border by its colour, or a blurred bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug, strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum Class {
    Black,
    White,
    Color,
    Blur,
}

impl Class {
    /// The name in the catalog (`border_sides.class`).
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<Class> {
        key.parse().ok()
    }

    /// The eval viewer's buckets: black / near-black up to 40, white / near-white from 215.
    fn of(kind: Kind, color: [u8; 3]) -> Class {
        match kind {
            Kind::Blur => Class::Blur,
            Kind::Solid if color.iter().all(|&c| c <= 40) => Class::Black,
            Kind::Solid if color.iter().all(|&c| c >= 215) => Class::White,
            Kind::Solid => Class::Color,
        }
    }
}

/// Why a candidate was rejected.
#[derive(Clone, Copy, PartialEq, Eq, Debug, strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum Reason {
    /// The run of border lines reaches the deepest a border may be.
    TooDeep,
    /// Too thin a run to measure the seam.
    NoSeam,
    SeamNotStraight,
    /// A smooth sky above a horizon, a plain wall beside a door frame: padding tools centre the content.
    UnpairedBlur,
    /// Shading: a wall or a sky.
    NotFlat,
    /// The accepted sides together would leave less than `min_keep` of the image.
    TooLittleLeft,
}

impl Reason {
    /// The name in the catalog (`border_sides.rejected`).
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<Reason> {
        key.parse().ok()
    }
}

/// A border found on one side, accepted or not.
#[derive(Clone, Debug, PartialEq)]
pub struct Side {
    /// The peeling round it was found in (0 first; nested borders come later).
    pub pass: u8,
    pub edge: Edge,
    pub kind: Kind,
    /// The content starts this many pixels from the edge (of what was left by earlier rounds).
    pub depth: u32,
    /// Solid: the line colour at the edge.
    pub color: Option<[u8; 3]>,
    /// Fraction of the seam that is a sharp straight edge.
    pub straight: f64,
    /// Solid and measured: median pixel deviation from the border colour, and how far the line colour wanders.
    pub dev: Option<f64>,
    pub drift: Option<f64>,
    /// `None`: accepted.
    pub rejected: Option<Reason>,
}

impl Side {
    pub fn accepted(&self) -> bool {
        self.rejected.is_none()
    }

    pub fn class(&self) -> Class {
        Class::of(self.kind, self.color.unwrap_or([128; 3]))
    }
}

/// What `borders::detect` (core) found in an image of `width` × `height` (stored pixels).
#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    pub width: u32,
    pub height: u32,
    /// The content box: left, top, right, bottom (right and bottom exclusive). The whole image when nothing was
    /// accepted.
    pub content: [u32; 4],
    /// Every side found, in the order found: accepted ones, and rejected candidates.
    pub sides: Sides,
}

/// The most sides a detection finds: `borders::detect` peels at most 3 rounds of the 4 edges, and finds at most one side
/// of an edge in a round.
pub const MAX_SIDES: usize = 12;

/// A file's sides, held inline: what detection finds, and what the catalog keeps of it.
pub type Sides = arrayvec::ArrayVec<Side, MAX_SIDES>;

impl Detection {
    /// Nothing found in an image of `width` × `height`.
    pub fn none(width: u32, height: u32) -> Detection {
        Detection { width, height, content: [0, 0, width, height], sides: Sides::new() }
    }

    pub fn found(&self) -> bool {
        self.sides.iter().any(Side::accepted)
    }

    /// Pixels cut from the left, top, right and bottom.
    pub fn cut(&self) -> [u32; 4] {
        let [l, t, r, b] = self.content;
        [l, t, self.width - r, self.height - b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    /// The names the catalog stores: a change to any of them is a change to the catalog's data.
    #[test]
    fn the_keys_are_the_catalog_names() {
        assert_eq!(Edge::iter().map(Edge::key).collect::<Vec<_>>(), ["top", "bottom", "left", "right"]);
        assert_eq!(Kind::iter().map(Kind::key).collect::<Vec<_>>(), ["solid", "blur"]);
        assert_eq!(Class::iter().map(Class::key).collect::<Vec<_>>(), ["black", "white", "color", "blur"]);
        assert_eq!(
            Reason::iter().map(Reason::key).collect::<Vec<_>>(),
            ["too_deep", "no_seam", "seam_not_straight", "unpaired_blur", "not_flat", "too_little_left"],
        );
        for e in Edge::iter() {
            assert_eq!(Edge::from_key(e.key()), Some(e));
        }
        assert_eq!(Reason::from_key("nope"), None);
    }
}
