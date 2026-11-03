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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right];

    pub fn key(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Bottom => "bottom",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }

    pub fn from_key(key: &str) -> Option<Edge> {
        Edge::ALL.into_iter().find(|e| e.key() == key)
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Solid,
    Blur,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Solid => "solid",
            Kind::Blur => "blur",
        }
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        [Kind::Solid, Kind::Blur].into_iter().find(|k| k.key() == key)
    }
}

/// What a side looks like, for the filters: a solid border by its colour, or a blurred bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Black,
    White,
    Color,
    Blur,
}

impl Class {
    pub const ALL: [Class; 4] = [Class::Black, Class::White, Class::Color, Class::Blur];

    pub fn key(self) -> &'static str {
        match self {
            Class::Black => "black",
            Class::White => "white",
            Class::Color => "color",
            Class::Blur => "blur",
        }
    }

    pub fn from_key(key: &str) -> Option<Class> {
        Class::ALL.into_iter().find(|c| c.key() == key)
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    const ALL: [Reason; 6] = [
        Reason::TooDeep,
        Reason::NoSeam,
        Reason::SeamNotStraight,
        Reason::UnpairedBlur,
        Reason::NotFlat,
        Reason::TooLittleLeft,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Reason::TooDeep => "too_deep",
            Reason::NoSeam => "no_seam",
            Reason::SeamNotStraight => "seam_not_straight",
            Reason::UnpairedBlur => "unpaired_blur",
            Reason::NotFlat => "not_flat",
            Reason::TooLittleLeft => "too_little_left",
        }
    }

    pub fn from_key(key: &str) -> Option<Reason> {
        Reason::ALL.into_iter().find(|r| r.key() == key)
    }

    /// The prototype's words for it.
    pub fn text(self) -> &'static str {
        match self {
            Reason::TooDeep => "too deep",
            Reason::NoSeam => "no seam",
            Reason::SeamNotStraight => "seam not straight",
            Reason::UnpairedBlur => "blurred bar without an equal opposite bar",
            Reason::NotFlat => "not flat (shading: wall or sky?)",
            Reason::TooLittleLeft => "too little content left",
        }
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

    /// The prototype's `Side.describe()`, for reports compared with its own.
    pub fn describe(&self) -> String {
        let kind = match (self.kind, self.color) {
            (Kind::Solid, Some([r, g, b])) => format!(
                "solid #{r:02x}{g:02x}{b:02x} dev {:.0} drift {:.0}",
                self.dev.unwrap_or(0.0),
                self.drift.unwrap_or(0.0)
            ),
            _ => "blur".to_string(),
        };
        let verdict = self.rejected.map(|r| format!(" [{}]", r.text())).unwrap_or_default();
        format!("{}: {kind} {}px straight {:.2}{verdict}", self.edge.key(), self.depth, self.straight)
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
