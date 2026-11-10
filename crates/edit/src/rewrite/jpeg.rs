//! JPEG, turned or cropped in the DCT domain.

use anyhow::Result;
use turbojpeg::{Transform, TransformCrop, TransformOp, Transformer};

use super::op::Op;

/// A JPEG transform that can't be done perfectly, so the file is left as it is and the orientation tag carries the
/// turn (`rotate`). Every other error is a failure.
#[derive(Debug)]
pub struct Imperfect(pub String);

impl std::fmt::Display for Imperfect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not a perfect transform: {}", self.0)
    }
}

impl std::error::Error for Imperfect {}

/// In the DCT domain (libjpeg-turbo's lossless transform), every marker copied. Turns are perfect transforms only:
/// one that would have to drop or smear partial edge blocks is [`Imperfect`]. A crop's left and top must be on the
/// MCU grid ([`mcu`]).
pub(crate) fn rewrite(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    let mut t = Transform::default();
    match op {
        Op::Orient(o) => {
            t.op = match o.get() {
                2 => TransformOp::Hflip,
                3 => TransformOp::Rot180,
                4 => TransformOp::Vflip,
                5 => TransformOp::Transpose,
                6 => TransformOp::Rot90,
                7 => TransformOp::Transverse,
                8 => TransformOp::Rot270,
                _ => return Ok(bytes.to_vec()),
            };
            t.perfect = true;
        }
        Op::Crop([x, y, w, h]) => t.crop = Some(TransformCrop { x, y, width: Some(w), height: Some(h) }),
    }
    // Keep the entropy coding kind: progressive stays progressive; baseline gets optimized Huffman tables (the
    // transform can't reuse the original ones).
    // A JPEG whose structure is damaged before its scan is refused: the transform would re-encode what is left.
    let Some(progressive) = pg_container::jpeg_is_progressive(bytes) else {
        return Err(Imperfect("the JPEG's structure is damaged before its scan".into()).into());
    };
    t.progressive = progressive;
    t.optimize = !progressive;
    let perfect = t.perfect;
    match Transformer::new()?.transform_to_vec(&t, bytes) {
        Ok(out) => Ok(out),
        // A perfect transform fails when it would have to cut through a block: that is imperfect, not a failure.
        Err(e) if perfect => Err(Imperfect(e.to_string()).into()),
        Err(e) => Err(e.into()),
    }
}

/// The JPEG's minimum coded unit, width and height: the grid a lossless crop starts on.
pub(crate) fn mcu(bytes: &[u8]) -> Result<(usize, usize)> {
    let header = turbojpeg::read_header(bytes)?;
    Ok((header.subsamp.mcu_width(), header.subsamp.mcu_height()))
}
