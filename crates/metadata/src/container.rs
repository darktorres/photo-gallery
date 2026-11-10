//! Where each container keeps the metadata the import reads, found in one walk of the file: the EXIF block, the
//! XMP packets (an extended JPEG one reassembled, a compressed PNG one inflated, within a budget), and the frame
//! count.

use std::ops::Range;

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
use pg_container::{
    ExtendedChunk, GifBlock, add_extended_chunk, chunk_data, find, gif_blocks, jpeg_exif, jpeg_segments, png_chunks,
    range_in, webp_chunks, webp_exif,
};
use pg_model::format::Format;

use crate::xmp::{MetaScratch, Packet};

pub(crate) const JPEG_XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// Where a file's container keeps the metadata the import reads, found in one walk of it. Its XMP packets, main one
/// first, are in [`MetaScratch::packets`]: in the file, but for a reassembled extended JPEG packet and a compressed
/// PNG one, in [`MetaScratch::assembled`]. (A TIFF file's is in its IFD0, read with the EXIF block.)
pub(crate) struct Container {
    /// The EXIF block (a TIFF structure).
    pub(crate) exif: Option<Range<usize>>,
    pub(crate) frames: u32,
}

impl Container {
    pub(crate) fn walk(bytes: &[u8], format: Format, scratch: &mut MetaScratch) -> Container {
        let mut c = Container { exif: None, frames: 1 };
        let MetaScratch { packets, assembled, chunks, inflater, .. } = scratch;
        packets.clear();
        assembled.clear();
        match format {
            Format::Jpeg => {
                chunks.clear();
                for s in jpeg_segments(bytes).filter(|s| s.marker == 0xE1) {
                    if c.exif.is_none() {
                        c.exif = jpeg_exif(&s);
                    }
                    if let Some(packet) = s.data.strip_prefix(JPEG_XMP) {
                        packets.push(Packet::InFile(range_in(bytes, packet)));
                    } else {
                        add_extended_chunk(bytes, s.data, chunks);
                    }
                }
                // The last standard packet first, then the one before it.
                packets.reverse();
                extended_packets(bytes, chunks, assembled, packets);
            }
            Format::Png => {
                // acTL counts before the first IDAT.
                let (mut image_data, mut animation) = (false, None);
                for chunk in png_chunks(bytes) {
                    match &chunk.kind {
                        b"eXIf" if c.exif.is_none() => c.exif = Some(chunk_data(&chunk)),
                        b"iTXt" => match png_itxt_xmp(chunk.data) {
                            Some(ItxtXmp::Plain(text)) => packets.push(Packet::InFile(range_in(bytes, text))),
                            Some(ItxtXmp::Compressed(text)) => {
                                let inflater = inflater.get_or_insert_with(Box::default);
                                // The budget is for the file: what its earlier packets left of it.
                                let left = MAX_INFLATED.saturating_sub(assembled.len());
                                packets.extend(inflate_zlib(text, inflater, assembled, left).map(Packet::Assembled));
                            }
                            None => {}
                        },
                        b"IDAT" => image_data = true,
                        b"acTL" if !image_data && animation.is_none() => {
                            animation = Some(chunk.data.first_chunk::<4>().map(|&n| u32::from_be_bytes(n)));
                        }
                        _ => {}
                    }
                }
                c.frames = animation.flatten().unwrap_or(1).max(1);
            }
            Format::Webp => {
                let (mut animated, mut frames) = (false, 0);
                for chunk in webp_chunks(bytes) {
                    match &chunk.kind {
                        b"VP8X" => animated |= chunk.data.first().is_some_and(|f| f & 0x02 != 0),
                        b"ANMF" => frames += 1,
                        b"EXIF" if c.exif.is_none() => c.exif = Some(webp_exif(&chunk)),
                        b"XMP " => packets.push(Packet::InFile(chunk_data(&chunk))),
                        _ => {}
                    }
                }
                c.frames = if animated { frames.max(1) } else { 1 };
            }
            Format::Gif => {
                let mut blocks = gif_blocks(bytes);
                let (mut images, mut packet) = (0, None);
                for b in blocks.by_ref() {
                    match b {
                        GifBlock::Image { .. } => images += 1,
                        GifBlock::Extension { label: 0xFF, range } if packet.is_none() => {
                            packet = gif_xmp(bytes, range);
                        }
                        GifBlock::Extension { .. } => {}
                    }
                }
                // Only a GIF read to its trailer.
                if blocks.complete() {
                    c.frames = images.max(1);
                    packets.extend(packet.map(|x| Packet::InFile(range_in(bytes, x))));
                }
            }
            Format::Tiff => c.exif = Some(0..bytes.len()),
            Format::Bmp => {}
        }
        c
    }
}

/// The extended packets whose chunks are all there, by GUID, each reassembled into `assembled`. A packet's length
/// is what its first chunk found says; of chunks at the same offset, the last found counts; the chunks must follow
/// each other from offset 0 up to that length.
fn extended_packets(bytes: &[u8], chunks: &mut [ExtendedChunk], assembled: &mut Vec<u8>, packets: &mut Vec<Packet>) {
    chunks.sort_unstable_by(|a, b| {
        (bytes.get(a.guid.clone()), a.offset, a.seq).cmp(&(bytes.get(b.guid.clone()), b.offset, b.seq))
    });
    for packet in chunks.chunk_by(|a, b| bytes.get(a.guid.clone()) == bytes.get(b.guid.clone())) {
        let Some(full) = packet.iter().min_by_key(|c| c.seq).map(|c| c.full) else { continue };
        let start = assembled.len();
        for last in packet.chunk_by(|a, b| a.offset == b.offset).filter_map(<[_]>::last) {
            if last.offset != assembled.len() - start {
                break;
            }
            let Some(data) = bytes.get(last.data.clone()) else { break };
            assembled.extend_from_slice(data);
        }
        if assembled.len() - start == full {
            packets.push(Packet::Assembled(start..assembled.len()));
        } else {
            assembled.truncate(start);
        }
    }
}

/// The most the inflated XMP packets of one file may be, together. A packet is a few kilobytes; a larger one (a
/// compressed bomb in a small file), or one past what the file's earlier packets left, is treated as absent, so the
/// import doesn't grow to gigabytes for it, however many compressed chunks the file holds.
const MAX_INFLATED: usize = 16 << 20;

/// zlib data inflated onto the end of `out`, as `miniz_oxide::inflate::decompress_to_vec_zlib` would give it (the
/// same output, the same failures), with a kept decompressor. Its range in `out`; `None`, and `out` as it was, if it
/// doesn't inflate or inflates to more than `limit` bytes (the budget left of [`MAX_INFLATED`]; none left, it is
/// absent).
fn inflate_zlib(
    mut input: &[u8],
    decompressor: &mut DecompressorOxide,
    out: &mut Vec<u8>,
    limit: usize,
) -> Option<Range<usize>> {
    if limit == 0 {
        return None;
    }
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    decompressor.init();
    let start = out.len();
    out.resize(start + input.len().saturating_mul(2).min(limit), 0);
    let mut at = 0;
    loop {
        let (status, read, written) = decompress(decompressor, input, out.get_mut(start..)?, at, flags);
        at += written;
        // The input left: `None` if the decompressor says it read more than it was given.
        match (status, input.get(read..)) {
            (TINFLStatus::Done, _) => {
                out.truncate(start + at);
                return Some(start..out.len());
            }
            (TINFLStatus::HasMoreOutput, Some(rest)) if out.len() - start < limit => {
                input = rest;
                let len = out.len() - start;
                out.resize(start + len.saturating_mul(2).min(limit), 0);
            }
            _ => {
                out.truncate(start);
                return None;
            }
        }
    }
}

/// The text of an `XML:com.adobe.xmp` iTXt chunk.
enum ItxtXmp<'a> {
    Plain(&'a [u8]),
    /// zlib data.
    Compressed(&'a [u8]),
}

fn png_itxt_xmp(data: &[u8]) -> Option<ItxtXmp<'_>> {
    let rest = data.strip_prefix(b"XML:com.adobe.xmp\0")?;
    let (&compressed, rest) = rest.split_first()?;
    let rest = rest.get(1..)?; // compression method
    let lang_end = rest.iter().position(|&b| b == 0)?;
    let rest = rest.get(lang_end + 1..)?;
    let translated_end = rest.iter().position(|&b| b == 0)?;
    let text = rest.get(translated_end + 1..)?;
    Some(if compressed == 1 { ItxtXmp::Compressed(text) } else { ItxtXmp::Plain(text) })
}

/// The XMP packet of an application extension (`range`: the whole block), if it holds one.
fn gif_xmp(bytes: &[u8], range: Range<usize>) -> Option<&[u8]> {
    if bytes.get(range.start + 3..range.start + 14) != Some(b"XMP DataXMP") {
        return None;
    }
    let data = bytes.get(range.start + 14..range.end)?;
    let end = find(data, b"<?xpacket end=")?;
    let close = find(data.get(end..)?, b"?>")?;
    data.get(..end + close + 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::png_chunk;

    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// The next [`xorshift`] value, less than `n`.
    fn below(state: &mut u64, n: usize) -> usize {
        usize::try_from(xorshift(state) % u64::try_from(n).unwrap()).unwrap()
    }

    /// The next [`xorshift`] value's low byte.
    fn byte(state: &mut u64) -> u8 {
        xorshift(state).to_le_bytes()[0]
    }

    /// Extended packets are reassembled as the map of maps did it (the first chunk's length, the last chunk at an
    /// offset, contiguous from 0, by GUID): random chunk lists of up to 3 packets, chunks shuffled, repeated with other
    /// data, missing, or with another length.
    #[test]
    fn extended_packets_as_the_maps_made_them() {
        use std::collections::BTreeMap;
        let mut st = 0x853C49E6748FEA9Bu64;
        let mut scratch = MetaScratch::default();
        for _ in 0..3000 {
            let mut file = Vec::new();
            let mut segments: Vec<Vec<u8>> = Vec::new();
            for g in 0..1 + xorshift(&mut st) % 3 {
                let parity = u8::try_from(xorshift(&mut st) % 2).unwrap();
                let guid = [b'A' + (u8::try_from(g).unwrap() + parity); 32];
                let full = 1 + usize::try_from(xorshift(&mut st) % 40).unwrap();
                let packet: Vec<u8> = (0..full).map(|_| byte(&mut st)).collect();
                let mut at = 0;
                while at < full {
                    let bound = u64::try_from(full - at).unwrap();
                    let len = usize::try_from((1 + xorshift(&mut st) % 12).min(bound)).unwrap();
                    let r = xorshift(&mut st) % 10;
                    let (offset, said, data) = match r {
                        0 => (at + 1, full, packet[at..at + len].to_vec()),
                        1 => (at, full + 1, packet[at..at + len].to_vec()),
                        2 => (at, full, vec![0xEE; len]),
                        _ => (at, full, packet[at..at + len].to_vec()),
                    };
                    if r != 3 || !xorshift(&mut st).is_multiple_of(4) {
                        segments.push(
                            [
                                JPEG_XMP_EXTENSION,
                                &guid,
                                &u32::try_from(said).unwrap().to_be_bytes(),
                                &u32::try_from(offset).unwrap().to_be_bytes(),
                                &data,
                            ]
                            .concat(),
                        );
                    }
                    if r == 2 {
                        // The right data after the wrong: the last at an offset counts.
                        segments.push(
                            [
                                JPEG_XMP_EXTENSION,
                                &guid,
                                &u32::try_from(full).unwrap().to_be_bytes(),
                                &u32::try_from(at).unwrap().to_be_bytes(),
                                &packet[at..at + len],
                            ]
                            .concat(),
                        );
                    }
                    at += len;
                }
            }
            for i in (1..segments.len()).rev() {
                segments.swap(i, below(&mut st, i + 1));
            }
            for seg in &segments {
                file.extend_from_slice(seg);
            }
            // The old way.
            // GUID → (length, chunks by offset).
            type Chunks<'a> = (usize, BTreeMap<usize, &'a [u8]>);
            let mut maps: BTreeMap<&[u8], Chunks> = BTreeMap::new();
            let mut pos = 0;
            let mut ranges = Vec::new();
            for seg in &segments {
                ranges.push(pos..pos + seg.len());
                pos += seg.len();
            }
            for r in &ranges {
                let rest = &file[r.clone()][JPEG_XMP_EXTENSION.len()..];
                let full = pg_model::num::index(u32::from_be_bytes(rest[32..36].try_into().unwrap()));
                let offset = pg_model::num::index(u32::from_be_bytes(rest[36..40].try_into().unwrap()));
                maps.entry(&rest[..32]).or_insert((full, BTreeMap::new())).1.insert(offset, &rest[40..]);
            }
            let want: Vec<Vec<u8>> = maps
                .into_values()
                .filter_map(|(full, chunks)| {
                    let mut packet = Vec::new();
                    for (offset, data) in chunks {
                        if offset != packet.len() {
                            break;
                        }
                        packet.extend_from_slice(data);
                    }
                    (packet.len() == full).then_some(packet)
                })
                .collect();
            // The new.
            scratch.chunks.clear();
            scratch.assembled.clear();
            scratch.packets.clear();
            for r in &ranges {
                add_extended_chunk(&file, &file[r.clone()], &mut scratch.chunks);
            }
            extended_packets(&file, &mut scratch.chunks, &mut scratch.assembled, &mut scratch.packets);
            let got: Vec<Vec<u8>> = (scratch.packets.iter())
                .map(|p| match p {
                    Packet::Assembled(r) => scratch.assembled[r.clone()].to_vec(),
                    Packet::InFile(_) => panic!("an extended packet in the file"),
                })
                .collect();
            assert_eq!(got, want);
        }
    }

    /// A kept decompressor inflates as `decompress_to_vec_zlib` does: the same bytes, the same failures (truncated,
    /// corrupted, empty), each onto what `out` already holds.
    #[test]
    fn an_inflated_packet_over_the_limit_is_absent() {
        // A bomb: 16 MB of zeros packs to a few kilobytes, and inflates past the limit.
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; MAX_INFLATED + 1], 6);
        assert!(zlib.len() < 1 << 20, "{}", zlib.len());
        let mut out = b"before".to_vec();
        assert!(inflate_zlib(&zlib, &mut DecompressorOxide::default(), &mut out, MAX_INFLATED).is_none());
        assert_eq!(out, b"before");
        // Just within the limit: taken.
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; MAX_INFLATED], 6);
        assert!(inflate_zlib(&zlib, &mut DecompressorOxide::default(), &mut out, MAX_INFLATED).is_some());
        // No budget left: absent, and `out` as it was.
        let small = miniz_oxide::deflate::compress_to_vec_zlib(b"<x/>", 6);
        let before = out.len();
        assert!(inflate_zlib(&small, &mut DecompressorOxide::default(), &mut out, 0).is_none());
        assert_eq!(out.len(), before);
    }

    /// The budget is the file's, not each packet's: a PNG of many compressed iTXt chunks, each within the limit
    /// alone, inflates no more than [`MAX_INFLATED`] in all, and the packets past it are absent.
    #[test]
    fn many_compressed_packets_share_one_budget() {
        // Over 4 MiB in each, a few kilobytes packed: 3 fit in the budget, the rest don't.
        let packet = [&b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">"[..], &vec![b' '; 4 << 20], b"</x:xmpmeta>"].concat();
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&packet, 6);
        let itxt = [&b"XML:com.adobe.xmp\0"[..], &[1, 0], b"\0\0", &zlib].concat();
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
        for _ in 0..10 {
            png.extend(png_chunk(b"iTXt", &itxt));
        }
        png.extend(png_chunk(b"IEND", b""));
        let mut scratch = MetaScratch::default();
        Container::walk(&png, Format::Png, &mut scratch);
        assert!(scratch.assembled.len() <= MAX_INFLATED, "{}", scratch.assembled.len());
        let inflated = scratch.packets.iter().filter(|p| matches!(p, Packet::Assembled(_))).count();
        assert_eq!(inflated, MAX_INFLATED / packet.len());
    }

    #[test]
    fn inflate_as_decompress_to_vec_zlib() {
        let mut st = 0x2545F4914F6CDD1Du64;
        let mut decompressor = Box::<DecompressorOxide>::default();
        let mut out = b"before".to_vec();
        for i in 0..2000 {
            let len = usize::try_from(xorshift(&mut st) % 5000).unwrap();
            let repetitive = i % 2 == 0;
            let data: Vec<u8> =
                (0..len).map(|k| if repetitive { u8::try_from(k % 7).unwrap() } else { byte(&mut st) }).collect();
            let mut zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, u8::try_from(i % 10).unwrap());
            match i % 5 {
                1 if !zlib.is_empty() => zlib.truncate(below(&mut st, zlib.len())),
                2 if !zlib.is_empty() => {
                    let at = below(&mut st, zlib.len());
                    zlib[at] ^= 1 << (xorshift(&mut st) % 8);
                }
                3 => zlib.clear(),
                _ => {}
            }
            let want = miniz_oxide::inflate::decompress_to_vec_zlib(&zlib).ok();
            let before = out.len();
            let got = inflate_zlib(&zlib, &mut decompressor, &mut out, MAX_INFLATED).map(|r| out[r].to_vec());
            assert_eq!(got, want, "case {i}");
            if got.is_none() {
                assert_eq!(out.len(), before);
            }
            assert!(out.starts_with(b"before"));
        }
    }
}
