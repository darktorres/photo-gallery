//! All IO functionality needed for TIFF decoding
#[cfg(feature = "webp")]
use std::io::Cursor;
#[cfg(feature = "webp")]
use std::io::BufReader;
use std::io::{self, BufRead, Read, Seek, Take};

pub use crate::tags::ByteOrder;

/// Reader that is aware of the byte order.
#[derive(Debug)]
pub struct EndianReader<R> {
    reader: R,
    pub(crate) byte_order: ByteOrder,
}

impl<R: Read> EndianReader<R> {
    pub fn new(reader: R, byte_order: ByteOrder) -> Self {
        Self { reader, byte_order }
    }

    pub fn inner(&mut self) -> &mut R {
        &mut self.reader
    }

    pub fn goto_offset(&mut self, offset: u64) -> io::Result<()>
    where
        R: Seek,
    {
        self.reader.seek(io::SeekFrom::Start(offset))?;
        Ok(())
    }

    /// Reads an u16
    #[inline(always)]
    pub fn read_u16(&mut self) -> Result<u16, io::Error> {
        let mut n = [0u8; 2];
        self.reader.read_exact(&mut n)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => u16::from_le_bytes(n),
            ByteOrder::BigEndian => u16::from_be_bytes(n),
        })
    }

    /// Reads an i16
    #[inline(always)]
    pub fn read_i16(&mut self) -> Result<i16, io::Error> {
        let mut n = [0u8; 2];
        self.reader.read_exact(&mut n)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => i16::from_le_bytes(n),
            ByteOrder::BigEndian => i16::from_be_bytes(n),
        })
    }

    /// Reads an u32
    #[inline(always)]
    pub fn read_u32(&mut self) -> Result<u32, io::Error> {
        let mut n = [0u8; 4];
        self.reader.read_exact(&mut n)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => u32::from_le_bytes(n),
            ByteOrder::BigEndian => u32::from_be_bytes(n),
        })
    }

    /// Reads an i32
    #[inline(always)]
    pub fn read_i32(&mut self) -> Result<i32, io::Error> {
        let mut n = [0u8; 4];
        self.reader.read_exact(&mut n)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => i32::from_le_bytes(n),
            ByteOrder::BigEndian => i32::from_be_bytes(n),
        })
    }

    /// Reads an u64
    #[inline(always)]
    pub fn read_u64(&mut self) -> Result<u64, io::Error> {
        let mut n = [0u8; 8];
        self.reader.read_exact(&mut n)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => u64::from_le_bytes(n),
            ByteOrder::BigEndian => u64::from_be_bytes(n),
        })
    }

    /// Reads an i64
    #[inline(always)]
    pub fn read_i64(&mut self) -> Result<i64, io::Error> {
        let mut n = [0u8; 8];
        self.reader.read_exact(&mut n)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => i64::from_le_bytes(n),
            ByteOrder::BigEndian => i64::from_be_bytes(n),
        })
    }

    /// Reads an f32
    #[inline(always)]
    pub fn read_f32(&mut self) -> Result<f32, io::Error> {
        let mut n = [0u8; 4];
        self.reader.read_exact(&mut n)?;
        Ok(f32::from_bits(match self.byte_order {
            ByteOrder::LittleEndian => u32::from_le_bytes(n),
            ByteOrder::BigEndian => u32::from_be_bytes(n),
        }))
    }

    /// Reads an f64
    #[inline(always)]
    pub fn read_f64(&mut self) -> Result<f64, io::Error> {
        let mut n = [0u8; 8];
        self.reader.read_exact(&mut n)?;
        Ok(f64::from_bits(match self.byte_order {
            ByteOrder::LittleEndian => u64::from_le_bytes(n),
            ByteOrder::BigEndian => u64::from_be_bytes(n),
        }))
    }
}

//
// # READERS
//

/// photo-gallery: what the chunk readers keep from one chunk to the next, and through
/// [`super::Scratch`] from one image to the next: the decompressors, reset for each chunk, and the
/// buffers the chunks are read through. Upstream made all of them anew for each chunk.
#[derive(Default)]
pub(crate) struct ReaderScratch {
    /// The buffer of the LZW, Deflate and Group 4 readers' input (upstream: a `BufReader` each).
    pub(crate) input: Vec<u8>,
    #[cfg(feature = "lzw")]
    pub(crate) lzw: Option<weezl::decode::Decoder>,
    #[cfg(feature = "deflate")]
    pub(crate) deflate: Option<flate2::Decompress>,
    /// A Group 4 reader's line.
    #[cfg(feature = "fax")]
    pub(crate) line: Vec<u8>,
    /// A JPEG chunk's data, with the tables before it.
    #[cfg(feature = "jpeg")]
    pub(crate) jpeg_data: Vec<u8>,
    /// A JPEG chunk's samples.
    #[cfg(feature = "jpeg")]
    pub(crate) jpeg_samples: Vec<u8>,
    #[cfg(feature = "jpeg")]
    pub(crate) jpeg: zune_jpeg::Scratch,
}

impl ReaderScratch {
    pub(crate) fn trim(&mut self, bytes: usize) {
        trim(&mut self.input, bytes);
        #[cfg(feature = "fax")]
        trim(&mut self.line, bytes);
        #[cfg(feature = "jpeg")]
        {
            trim(&mut self.jpeg_data, bytes);
            trim(&mut self.jpeg_samples, bytes);
            self.jpeg.trim(bytes);
        }
    }
}

/// photo-gallery: frees `v` if it holds more than `bytes`.
pub(crate) fn trim(v: &mut Vec<u8>, bytes: usize) {
    if v.capacity() > bytes {
        *v = Vec::new();
    }
}

/// photo-gallery: `BufReader::with_capacity(capacity, inner)` in a kept buffer: it reads as std's does (a read at
/// least as large as the buffer, when the buffer is empty, goes past it; otherwise the buffer is filled with one
/// read of the inner reader when it is empty).
pub(crate) struct KeptBufReader<'s, R> {
    inner: R,
    buf: &'s mut Vec<u8>,
    pos: usize,
    filled: usize,
}

impl<'s, R: Read> KeptBufReader<'s, R> {
    pub(crate) fn new(inner: R, buf: &'s mut Vec<u8>, capacity: usize) -> Self {
        buf.clear();
        buf.resize(capacity, 0);
        KeptBufReader {
            inner,
            buf,
            pos: 0,
            filled: 0,
        }
    }
}

impl<R: Read> Read for KeptBufReader<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.filled && out.len() >= self.buf.len() {
            self.pos = 0;
            self.filled = 0;
            return self.inner.read(out);
        }
        let mut rem = self.fill_buf()?;
        let n = rem.read(out)?;
        self.consume(n);
        Ok(n)
    }
}

impl<R: Read> BufRead for KeptBufReader<'_, R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos >= self.filled {
            self.pos = 0;
            self.filled = 0;
            self.filled = self.inner.read(&mut self.buf[..])?;
        }
        Ok(&self.buf[self.pos..self.filled])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = (self.pos + amt).min(self.filled);
    }
}

//
// ## Deflate Reader
//

/// Reader that decompresses zlib streams.
///
/// photo-gallery: `flate2::read::ZlibDecoder` (upstream's reader, a type alias of it) over a kept
/// [`flate2::Decompress`], reset for each chunk (a full reset, which is what `Decompress::new`
/// makes), and a kept 32 KiB input buffer, its size there. `read` is flate2 1.1.10's.
#[cfg(feature = "deflate")]
pub struct DeflateReader<'s, R: Read> {
    reader: KeptBufReader<'s, R>,
    decompress: &'s mut flate2::Decompress,
}

#[cfg(feature = "deflate")]
impl<'s, R: Read> DeflateReader<'s, R> {
    pub(crate) fn new(reader: R, scratch: &'s mut ReaderScratch) -> Self {
        let decompress = match &mut scratch.deflate {
            Some(d) => {
                d.reset(true);
                d
            }
            none => none.insert(flate2::Decompress::new(true)),
        };
        DeflateReader {
            reader: KeptBufReader::new(reader, &mut scratch.input, 32 * 1024),
            decompress,
        }
    }
}

#[cfg(feature = "deflate")]
impl<R: Read> Read for DeflateReader<'_, R> {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        use flate2::{FlushDecompress, Status};
        loop {
            let (read, consumed, ret, eof);
            {
                let input = self.reader.fill_buf()?;
                eof = input.is_empty();
                let before_out = self.decompress.total_out();
                let before_in = self.decompress.total_in();
                let flush = if eof {
                    FlushDecompress::Finish
                } else {
                    FlushDecompress::None
                };
                ret = self.decompress.decompress(input, dst, flush);
                read = (self.decompress.total_out() - before_out) as usize;
                consumed = (self.decompress.total_in() - before_in) as usize;
            }
            self.reader.consume(consumed);

            match ret {
                Ok(Status::Ok | Status::BufError) if read == 0 && !eof && !dst.is_empty() => continue,
                Ok(Status::Ok | Status::BufError) if read == 0 && eof && !dst.is_empty() => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "incomplete deflate stream",
                    ));
                }
                Ok(Status::Ok | Status::BufError | Status::StreamEnd) => return Ok(read),
                Err(..) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "corrupt deflate stream",
                    ))
                }
            }
        }
    }
}

//
// ## LZW Reader
//

/// Reader that decompresses LZW streams
///
/// photo-gallery: in a kept decoder, reset for each chunk (weezl's `reset`, which gives a decoder as
/// `build` makes it), and a kept input buffer of the size upstream's `BufReader` had.
#[cfg(feature = "lzw")]
pub struct LZWReader<'s, R: Read> {
    reader: KeptBufReader<'s, Take<R>>,
    decoder: &'s mut weezl::decode::Decoder,
}

#[cfg(feature = "lzw")]
impl<'s, R: Read> LZWReader<'s, R> {
    /// Wraps a reader
    pub(crate) fn new(
        reader: R,
        compressed_length: usize,
        scratch: &'s mut ReaderScratch,
    ) -> LZWReader<'s, R> {
        let decoder = match &mut scratch.lzw {
            Some(d) => {
                d.reset();
                d
            }
            none => none.insert(
                weezl::decode::Configuration::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
                    .with_yield_on_full_buffer(true)
                    .build(),
            ),
        };
        Self {
            reader: KeptBufReader::new(
                reader.take(u64::try_from(compressed_length).unwrap()),
                &mut scratch.input,
                (32 * 1024).min(compressed_length),
            ),
            decoder,
        }
    }
}

#[cfg(feature = "lzw")]
impl<R: Read> Read for LZWReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let result = self.decoder.decode_bytes(self.reader.fill_buf()?, buf);
            self.reader.consume(result.consumed_in);

            match result.status {
                Ok(weezl::LzwStatus::Ok) => {
                    if result.consumed_out == 0 {
                        continue;
                    } else {
                        return Ok(result.consumed_out);
                    }
                }
                Ok(weezl::LzwStatus::NoProgress) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "no lzw end code found",
                    ));
                }
                Ok(weezl::LzwStatus::Done) => {
                    return Ok(result.consumed_out);
                }
                Err(err) => return Err(io::Error::new(io::ErrorKind::InvalidData, err)),
            }
        }
    }
}

//
// ## PackBits Reader
//

/// Internal state machine for the PackBitsReader.
enum PackBitsReaderState {
    Header,
    Literal,
    Repeat { value: u8 },
}

/// Reader that unpacks Apple's `PackBits` format
pub struct PackBitsReader<R: Read> {
    reader: Take<R>,
    state: PackBitsReaderState,
    count: usize,
}

impl<R: Read> PackBitsReader<R> {
    /// Wraps a reader
    pub fn new(reader: R, length: u64) -> Self {
        Self {
            reader: reader.take(length),
            state: PackBitsReaderState::Header,
            count: 0,
        }
    }
}

impl<R: Read> Read for PackBitsReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while let PackBitsReaderState::Header = self.state {
            if self.reader.limit() == 0 {
                return Ok(0);
            }
            let mut header: [u8; 1] = [0];
            self.reader.read_exact(&mut header)?;
            let h = header[0] as i8;
            if (-127..=-1).contains(&h) {
                let mut data: [u8; 1] = [0];
                self.reader.read_exact(&mut data)?;
                self.state = PackBitsReaderState::Repeat { value: data[0] };
                self.count = (1 - h as isize) as usize;
            } else if h >= 0 {
                self.state = PackBitsReaderState::Literal;
                self.count = h as usize + 1;
            } else {
                // h = -128 is a no-op.
            }
        }

        let length = buf.len().min(self.count);
        let actual = match self.state {
            PackBitsReaderState::Literal => self.reader.read(&mut buf[..length])?,
            PackBitsReaderState::Repeat { value } => {
                for b in &mut buf[..length] {
                    *b = value;
                }

                length
            }
            PackBitsReaderState::Header => unreachable!(),
        };

        self.count -= actual;
        if self.count == 0 {
            self.state = PackBitsReaderState::Header;
        }
        Ok(actual)
    }
}

/// photo-gallery: its input buffer (8 KiB, `BufReader::new`'s) and its line kept (upstream: new ones
/// for each chunk).
#[cfg(feature = "fax")]
pub struct Group4Reader<'s, R: Read> {
    decoder: fax34::decoder::Group4Decoder<io::Bytes<KeptBufReader<'s, io::Take<R>>>>,
    line_buf: io::Cursor<&'s mut Vec<u8>>,
    height: u16,
    width: u16,
    y: u16,
}

#[cfg(feature = "fax")]
impl<'s, R: Read> Group4Reader<'s, R> {
    pub(crate) fn new(
        dimensions: (u32, u32),
        reader: R,
        compressed_length: u64,
        scratch: &'s mut ReaderScratch,
    ) -> crate::TiffResult<Self> {
        let width = u16::try_from(dimensions.0)?;
        let height = u16::try_from(dimensions.1)?;
        scratch.line.clear();

        Ok(Self {
            decoder: fax34::decoder::Group4Decoder::new(
                KeptBufReader::new(reader.take(compressed_length), &mut scratch.input, 8 * 1024)
                    .bytes(),
                width,
            )?,
            line_buf: io::Cursor::new(&mut scratch.line),
            width,
            height,
            y: 0,
        })
    }
}

#[cfg(feature = "fax")]
impl<R: Read> Read for Group4Reader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Either we have not read any line or we are at the end of a line.
        if self.line_buf.position() as usize == self.line_buf.get_ref().len()
            && self.y < self.height
        {
            let next = self.decoder.advance().map_err(std::io::Error::other)?;

            match next {
                fax34::decoder::DecodeStatus::End => (),
                fax34::decoder::DecodeStatus::Incomplete => {
                    self.y += 1;

                    // We known `transitions` yields exactly `self.width` items (per doc).
                    // FIXME: performance. We do not need an individual pixel iterator, filling
                    // memory especially for long runs can be much quicker. The `transitions` are
                    // the positions at which each run-length ends, i.e. the prefix sum of run
                    // lengths. Runs in fax4 start with white.
                    let transitions = fax34::decoder::pels(self.decoder.transition(), self.width);

                    let buffer = self.line_buf.get_mut();
                    buffer.resize(usize::from(self.width).div_ceil(8), 0u8);

                    let target = &mut buffer[..];

                    // Note: it may seem strange to treat black as 0b1 and white as 0b0 despite all
                    // our streams by default decoding as-if PhotometricInterpretation::BlackIsMin.
                    // This is however consistent with libtiff. It seems that fax4's "White"
                    // differs from what libtiff thinks of as "White". For content, a line of data
                    // is in runlength encoding of white-black-white-black always starting with
                    // white. In libtiff, the loop always does both colors in one go and the
                    // structure is:
                    //
                    // ```
                    // for (; runs < erun; runs += 2)
                    //   // white run
                    //       do { *lp++ = 0L; } while (…)
                    //   // black run
                    //       do { *lp++ = -1L; } while (…)
                    // ```
                    //
                    // So indeed the Fax4::White run is implemented by filling with zeros.
                    let mut bits = transitions.map(|c| match c {
                        fax34::Color::Black => true,
                        fax34::Color::White => false,
                    });

                    // Assemble bits in MSB as per our library representation for buffer.
                    for byte in target {
                        let mut val = 0;

                        for (idx, bit) in bits.by_ref().take(8).enumerate() {
                            val |= u8::from(bit) << (7 - idx % 8);
                        }

                        *byte = val;
                    }

                    self.line_buf.set_position(0);
                }
            }
        }

        self.line_buf.read(buf)
    }
}

#[cfg(feature = "webp")]
pub struct WebPReader {
    inner: Cursor<Vec<u8>>,
}

#[cfg(feature = "webp")]
impl WebPReader {
    pub fn new<R: Read + Seek>(
        reader: R,
        compressed_length: u64,
        samples: u16,
    ) -> crate::TiffResult<Self> {
        let mut decoder =
            image_webp::WebPDecoder::new(io::BufReader::new(reader.take(compressed_length)))
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        if !(samples == 4 || (samples == 3 && !decoder.has_alpha())) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad sample count for WebP compressed data",
            )
            .into());
        }

        let total_bytes =
            samples as usize * decoder.dimensions().0 as usize * decoder.dimensions().1 as usize;
        let mut data = vec![0; total_bytes];

        decoder
            .read_image(&mut data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Add a fully opaque alpha channel if needed
        if samples == 4 && !decoder.has_alpha() {
            for i in (0..(total_bytes / 4)).rev() {
                data[i * 4 + 3] = 255;
                data[i * 4 + 2] = data[i * 3 + 2];
                data[i * 4 + 1] = data[i * 3 + 1];
                data[i * 4] = data[i * 3];
            }
        }

        Ok(Self {
            inner: Cursor::new(data),
        })
    }
}

#[cfg(feature = "webp")]
impl Read for WebPReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_packbits() {
        let encoded = vec![
            0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7,
            0xAA,
        ];
        let encoded_len = encoded.len();

        let buff = io::Cursor::new(encoded);
        let mut decoder = PackBitsReader::new(buff, encoded_len as u64);

        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();

        let expected = vec![
            0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
        ];
        assert_eq!(decoded, expected);
    }
}
