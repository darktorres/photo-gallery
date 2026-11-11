//! FFmpeg, loaded at run time. The libraries are opened by name (or, on Windows, by full path in the folder that holds
//! them, with the search limited to that folder and the system's), and each function is looked up by name at first
//! use. Nothing is linked, so no search path (PATH, the working folder) decides which FFmpeg runs, and a missing one is
//! an error from [`api`], not a failure to start.
//!
//! The types and constants come from FFmpeg's headers (build.rs), so their layouts are FFmpeg's. The functions'
//! signatures are written from the same headers; the calls go through the pointers in [`Api`]. The loader is `load`,
//! the file an input reads through FFmpeg's I/O is `io`, and the safe wrappers of the C API are here.

// The generated names are FFmpeg's, and its enum values are constants (build.rs). The layout tests are off.
#[expect(
    non_camel_case_types,
    non_upper_case_globals,
    dead_code,
    unreachable_pub,
    clippy::type_complexity,
    reason = "FFmpeg's own names, from its headers, every constant of the enums they come with, the callback types \
              of its structs (AVCodecContext's execute2) as bindgen writes them, and the `pub` bindgen puts on \
              every item (the crate takes them as `pub(crate)`)"
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/ffmpeg.rs"));
}
pub(crate) use generated::*;

mod load;
pub(crate) use load::{Api, api};
mod io;
use io::{IO_BUFFER, Source, free_io, hint_name, read_source, seek_source};

use std::ffi::{CStr, CString, c_char, c_int};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr;

use anyhow::{Result, bail};

/// `AV_NOPTS_VALUE`: no timestamp. The headers define it as a macro, which bindgen doesn't emit.
pub(crate) const NOPTS_VALUE: i64 = i64::MIN;

/// What FFmpeg says about error `code`.
pub(crate) fn error_text(code: c_int) -> String {
    let Ok(api) = api() else {
        return format!("error {code}");
    };
    let mut buf: [c_char; 256] = [0; 256];
    // SAFETY: the buffer is 256 bytes, and av_strerror writes a NUL-terminated string into it.
    unsafe {
        (api.av_strerror)(code, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// The codec's name (`h264`, `hevc`, `av1`…) for its id: FFmpeg's own, static text.
pub(crate) fn codec_name(id: AVCodecID) -> &'static str {
    let Ok(api) = api() else {
        return "";
    };
    // SAFETY: avcodec_get_name returns a pointer to a static, NUL-terminated name.
    unsafe {
        let name = (api.avcodec_get_name)(id);
        if name.is_null() { "" } else { CStr::from_ptr(name).to_str().unwrap_or_default() }
    }
}

/// The value of metadata `key` in dictionary `dict` (null if there is none).
fn dict_get(api: &Api, dict: *const AVDictionary, key: &str) -> Option<String> {
    let key = CString::new(key).ok()?;
    // SAFETY: `dict` is a dictionary FFmpeg made (or null, which av_dict_get accepts), and `key` is NUL-terminated.
    unsafe {
        let entry = (api.av_dict_get)(dict, key.as_ptr(), ptr::null(), 0);
        if entry.is_null() || (*entry).value.is_null() {
            return None;
        }
        Some(CStr::from_ptr((*entry).value).to_string_lossy().into_owned())
    }
}

/// An opened file, closed on drop.
pub(crate) struct Input {
    ptr: *mut AVFormatContext,
    /// FFmpeg's I/O over `source`, freed on drop after the format context.
    io: *mut AVIOContext,
    source: *mut Source,
    api: &'static Api,
}

impl Input {
    /// Opens the file at `path` and reads its stream information. The file is read through a handle of its own, so
    /// its bytes are the file's, whatever the name (a lone surrogate, a raw byte) and however long the path. `path` is
    /// opened as given: a caller passes the verbatim form (`pg_platform::native::verbatim`), so a folder name that
    /// Windows would change (a trailing dot or space) reaches the file it names and not a sibling's.
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let api = api()?;
        let file = std::fs::File::open(path).map_err(crate::unopened::Unopened)?;
        let len = file.metadata().map_err(crate::unopened::Unopened)?.len();
        let source = Box::into_raw(Box::new(Source { file, len }));
        // SAFETY: `av_malloc` gives a buffer of `IO_BUFFER` bytes; the I/O context takes it over (see `free_io`).
        let buffer = unsafe { (api.av_malloc)(IO_BUFFER) }.cast::<u8>();
        if buffer.is_null() {
            // SAFETY: `source` came from `Box::into_raw` above, and nothing else holds it.
            unsafe { drop(Box::from_raw(source)) };
            bail!("out of memory");
        }
        let size = c_int::try_from(IO_BUFFER).unwrap_or(c_int::MAX);
        // SAFETY: `buffer` is `size` bytes; `source` is the opaque pointer the callbacks read, alive while the context is
        // (`free_io` frees both, after the context). The context has no write callback: the input is read only.
        let io = unsafe {
            (api.avio_alloc_context)(buffer, size, 0, source.cast(), Some(read_source), None, Some(seek_source))
        };
        if io.is_null() {
            // SAFETY: the context failed without taking the buffer, so the buffer and the source are still ours.
            unsafe {
                (api.av_free)(buffer.cast());
                drop(Box::from_raw(source));
            }
            bail!("out of memory");
        }
        // SAFETY: a new format context, not yet opened.
        let ctx = unsafe { (api.avformat_alloc_context)() };
        if ctx.is_null() {
            free_io(api, io, source);
            bail!("out of memory");
        }
        // SAFETY: `ctx` is the context made above, not opened yet; its I/O is `io`, and the custom-I/O flag keeps
        // FFmpeg from closing it.
        unsafe {
            (*ctx).pb = io;
            (*ctx).flags |= c_int::try_from(AVFMT_FLAG_CUSTOM_IO).unwrap_or_default();
        }
        let mut ptr = ctx;
        let name = hint_name(path);
        // SAFETY: `ptr` receives the opened context; `name` is NUL-terminated and outlives the call. On failure FFmpeg
        // frees the context and sets `ptr` to null, so only the I/O is left to free.
        let ret = unsafe { (api.avformat_open_input)(&mut ptr, name.as_ptr(), ptr::null(), ptr::null_mut()) };
        if ret < 0 {
            free_io(api, io, source);
            bail!("{}", error_text(ret));
        }
        let input = Self { ptr, io, source, api };
        // SAFETY: `input.ptr` is the open context; its drop closes it.
        let ret = unsafe { (api.avformat_find_stream_info)(input.ptr, ptr::null_mut()) };
        if ret < 0 {
            bail!("{}", error_text(ret));
        }
        Ok(input)
    }

    /// The best video stream (FFmpeg's choice), if there is one.
    pub(crate) fn best_video(&self) -> Option<Stream<'_>> {
        // SAFETY: `self.ptr` is an open context; the decoder out-parameter is allowed to be null.
        let index = unsafe {
            (self.api.av_find_best_stream)(self.ptr, AVMediaType_AVMEDIA_TYPE_VIDEO, -1, -1, ptr::null_mut(), 0)
        };
        let index = usize::try_from(index).ok()?;
        // SAFETY: `self.ptr` is open, so its streams array has `nb_streams` entries, and `index` is below that.
        unsafe {
            if index >= usize::try_from((*self.ptr).nb_streams).ok()? {
                return None;
            }
            let stream = *(*self.ptr).streams.add(index);
            if stream.is_null() {
                return None;
            }
            Some(Stream { ptr: stream, api: self.api, _input: PhantomData })
        }
    }

    /// The container's duration, in AV_TIME_BASE (µs).
    pub(crate) fn duration(&self) -> i64 {
        // SAFETY: `self.ptr` is open.
        unsafe { (*self.ptr).duration }
    }

    /// The container's metadata under `key`.
    pub(crate) fn metadata(&self, key: &str) -> Option<String> {
        // SAFETY: `self.ptr` is open.
        dict_get(self.api, unsafe { (*self.ptr).metadata }, key)
    }

    /// Seeks to the keyframe at or before `ts` (µs). Returns whether it could.
    pub(crate) fn seek(&mut self, ts: i64) -> bool {
        // SAFETY: `self.ptr` is open; stream -1 means the container's time base (µs).
        unsafe { (self.api.avformat_seek_file)(self.ptr, -1, i64::MIN, ts, ts, 0) >= 0 }
    }

    /// The next packet, into `packet` (its old contents are dropped first). False at the end, or on a read error.
    pub(crate) fn read_packet(&mut self, packet: &mut Packet) -> bool {
        packet.unref();
        // SAFETY: `self.ptr` is open and `packet.ptr` is a packet from av_packet_alloc.
        unsafe { (self.api.av_read_frame)(self.ptr, packet.ptr) >= 0 }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        // SAFETY: `self.ptr` came from avformat_open_input and is closed once, here (it doesn't close the custom I/O).
        unsafe { (self.api.avformat_close_input)(&mut self.ptr) }
        free_io(self.api, self.io, self.source);
    }
}

/// A stream of an [`Input`], borrowed from it.
pub(crate) struct Stream<'a> {
    ptr: *mut AVStream,
    api: &'static Api,
    _input: PhantomData<&'a Input>,
}

impl Stream<'_> {
    pub(crate) fn index(&self) -> c_int {
        // SAFETY: the stream belongs to the open input, which outlives this borrow.
        unsafe { (*self.ptr).index }
    }

    pub(crate) fn time_base(&self) -> AVRational {
        // SAFETY: as above.
        unsafe { (*self.ptr).time_base }
    }

    pub(crate) fn start_time(&self) -> i64 {
        // SAFETY: as above.
        unsafe { (*self.ptr).start_time }
    }

    pub(crate) fn duration(&self) -> i64 {
        // SAFETY: as above.
        unsafe { (*self.ptr).duration }
    }

    pub(crate) fn frames(&self) -> i64 {
        // SAFETY: as above.
        unsafe { (*self.ptr).nb_frames }
    }

    pub(crate) fn avg_frame_rate(&self) -> AVRational {
        // SAFETY: as above.
        unsafe { (*self.ptr).avg_frame_rate }
    }

    pub(crate) fn codec_id(&self) -> AVCodecID {
        // SAFETY: as above; codecpar is set for every stream.
        unsafe { (*(*self.ptr).codecpar).codec_id }
    }

    pub(crate) fn metadata(&self, key: &str) -> Option<String> {
        // SAFETY: as above.
        dict_get(self.api, unsafe { (*self.ptr).metadata }, key)
    }

    /// The display matrix (nine 16.16 fixed-point words, little-endian) the stream's side data carries, if any. In
    /// FFmpeg 7 and later the side data is the codec parameters', not the stream's.
    pub(crate) fn display_matrix(&self) -> Option<[i32; 9]> {
        // SAFETY: as above; codecpar is set for every stream, and its `coded_side_data` has `nb_coded_side_data`
        // entries, each with `size` bytes at `data`.
        unsafe {
            let params = (*self.ptr).codecpar;
            let count = usize::try_from((*params).nb_coded_side_data).ok()?;
            // A null pointer is not a slice even when empty, so a stream without side data has none to look at.
            if count == 0 || (*params).coded_side_data.is_null() {
                return None;
            }
            let entries = std::slice::from_raw_parts((*params).coded_side_data, count);
            let side = entries.iter().find(|sd| sd.type_ == AVPacketSideDataType_AV_PKT_DATA_DISPLAYMATRIX)?;
            if side.data.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(side.data, side.size);
            // Nine 4-byte words; the `.0` drops the remainder, which is empty once `get(..36)` has checked the length.
            let words: Vec<i32> = bytes.get(..36)?.as_chunks::<4>().0.iter().map(|w| i32::from_le_bytes(*w)).collect();
            words.try_into().ok()
        }
    }
}

/// A packet, freed on drop.
pub(crate) struct Packet {
    ptr: *mut AVPacket,
    api: &'static Api,
}

impl Packet {
    pub(crate) fn new() -> Result<Self> {
        let api = api()?;
        // SAFETY: av_packet_alloc takes no arguments and returns a new packet or null.
        let ptr = unsafe { (api.av_packet_alloc)() };
        if ptr.is_null() {
            bail!("av_packet_alloc failed");
        }
        Ok(Self { ptr, api })
    }

    pub(crate) fn stream_index(&self) -> c_int {
        // SAFETY: `ptr` is a live packet.
        unsafe { (*self.ptr).stream_index }
    }

    fn unref(&mut self) {
        // SAFETY: `ptr` is a live packet.
        unsafe { (self.api.av_packet_unref)(self.ptr) }
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from av_packet_alloc and is freed once, here.
        unsafe { (self.api.av_packet_free)(&mut self.ptr) }
    }
}

/// A video decoder for a stream, freed on drop.
pub(crate) struct Decoder {
    ptr: *mut AVCodecContext,
    api: &'static Api,
}

impl Decoder {
    /// Opens the decoder for `stream`, with its parameters.
    pub(crate) fn open(stream: &Stream<'_>) -> Result<Self> {
        let api = stream.api;
        // SAFETY: the stream is open, so its codecpar is set.
        let params = unsafe { (*stream.ptr).codecpar };
        // SAFETY: `params` is the stream's, and the codec id is a value FFmpeg returns for its decoders.
        let codec = unsafe { (api.avcodec_find_decoder)((*params).codec_id) };
        if codec.is_null() {
            bail!("no decoder for {}", codec_name(stream.codec_id()));
        }
        // SAFETY: `codec` is FFmpeg's, and the new context is freed by Decoder's drop, from here on.
        let ptr = unsafe { (api.avcodec_alloc_context3)(codec) };
        if ptr.is_null() {
            bail!("avcodec_alloc_context3 failed");
        }
        let decoder = Self { ptr, api };
        // SAFETY: `decoder.ptr` and `codec` are valid, and `params` is the stream's, open while `stream` is.
        let ret = unsafe { (api.avcodec_parameters_to_context)(decoder.ptr, params) };
        if ret < 0 {
            bail!("{}", error_text(ret));
        }
        // SAFETY: as above; no options.
        let ret = unsafe { (api.avcodec_open2)(decoder.ptr, codec, ptr::null_mut()) };
        if ret < 0 {
            bail!("{}", error_text(ret));
        }
        Ok(decoder)
    }

    pub(crate) fn width(&self) -> u32 {
        // SAFETY: `ptr` is open.
        u32::try_from(unsafe { (*self.ptr).width }).unwrap_or(0)
    }

    pub(crate) fn height(&self) -> u32 {
        // SAFETY: `ptr` is open.
        u32::try_from(unsafe { (*self.ptr).height }).unwrap_or(0)
    }

    pub(crate) fn sample_aspect_ratio(&self) -> AVRational {
        // SAFETY: `ptr` is open.
        unsafe { (*self.ptr).sample_aspect_ratio }
    }

    /// Sends a packet to decode. False if the decoder refused it.
    pub(crate) fn send_packet(&mut self, packet: &Packet) -> bool {
        // SAFETY: `ptr` is open and `packet.ptr` is a live packet.
        unsafe { (self.api.avcodec_send_packet)(self.ptr, packet.ptr) == 0 }
    }

    /// Tells the decoder there are no more packets, so it outputs what it holds.
    pub(crate) fn send_eof(&mut self) {
        // SAFETY: a null packet is FFmpeg's end-of-stream.
        unsafe { (self.api.avcodec_send_packet)(self.ptr, ptr::null()) };
    }

    /// The next decoded frame, into `frame`. False when there is none (more input is needed, or the end).
    pub(crate) fn receive_frame(&mut self, frame: &mut Frame) -> bool {
        // SAFETY: `ptr` is open and `frame.ptr` is a live frame.
        unsafe { (self.api.avcodec_receive_frame)(self.ptr, frame.ptr) == 0 }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from avcodec_alloc_context3 and is freed once, here.
        unsafe { (self.api.avcodec_free_context)(&mut self.ptr) }
    }
}

/// A frame, freed on drop.
pub(crate) struct Frame {
    ptr: *mut AVFrame,
    api: &'static Api,
}

impl Frame {
    pub(crate) fn new() -> Result<Self> {
        let api = api()?;
        // SAFETY: av_frame_alloc takes no arguments and returns a new frame or null.
        let ptr = unsafe { (api.av_frame_alloc)() };
        if ptr.is_null() {
            bail!("av_frame_alloc failed");
        }
        Ok(Self { ptr, api })
    }

    /// A video frame of `format` and size `w` × `h`, with its buffers allocated.
    pub(crate) fn video(format: AVPixelFormat, w: u32, h: u32) -> Result<Self> {
        let frame = Self::new()?;
        let (cw, ch) = (i32::try_from(w)?, i32::try_from(h)?);
        // SAFETY: `frame.ptr` is a live frame; the buffers are allocated for its format and size.
        unsafe {
            (*frame.ptr).format = format;
            (*frame.ptr).width = cw;
            (*frame.ptr).height = ch;
            let ret = (frame.api.av_frame_get_buffer)(frame.ptr, 0);
            if ret < 0 {
                bail!("allocating the frame: {}", error_text(ret));
            }
        }
        Ok(frame)
    }

    pub(crate) fn width(&self) -> u32 {
        // SAFETY: `ptr` is a live frame.
        u32::try_from(unsafe { (*self.ptr).width }).unwrap_or(0)
    }

    pub(crate) fn height(&self) -> u32 {
        // SAFETY: `ptr` is a live frame.
        u32::try_from(unsafe { (*self.ptr).height }).unwrap_or(0)
    }

    pub(crate) fn sample_aspect_ratio(&self) -> AVRational {
        // SAFETY: as above.
        unsafe { (*self.ptr).sample_aspect_ratio }
    }

    pub(crate) fn color_space(&self) -> AVColorSpace {
        // SAFETY: as above.
        unsafe { (*self.ptr).colorspace }
    }

    pub(crate) fn color_range(&self) -> AVColorRange {
        // SAFETY: as above.
        unsafe { (*self.ptr).color_range }
    }

    pub(crate) fn set_color_space(&mut self, space: AVColorSpace) {
        // SAFETY: as above.
        unsafe { (*self.ptr).colorspace = space }
    }

    pub(crate) fn set_color_range(&mut self, range: AVColorRange) {
        // SAFETY: as above.
        unsafe { (*self.ptr).color_range = range }
    }

    /// The frame's presentation time (its best estimate), if it has one.
    pub(crate) fn timestamp(&self) -> Option<i64> {
        // SAFETY: as above.
        let t = unsafe { (*self.ptr).best_effort_timestamp };
        (t != NOPTS_VALUE).then_some(t)
    }

    /// The first plane's row stride and its bytes (`stride × height`): a packed picture, such as RGB24.
    pub(crate) fn packed_plane(&self) -> (usize, &[u8]) {
        // SAFETY: as above. The first plane's buffer holds `linesize[0] × height` bytes (av_frame_get_buffer's layout).
        unsafe {
            let stride = (*self.ptr).linesize.first().copied().and_then(|s| usize::try_from(s).ok()).unwrap_or(0);
            let data = (*self.ptr).data.first().copied().unwrap_or(ptr::null_mut());
            let len = stride * usize::try_from((*self.ptr).height).unwrap_or(0);
            if data.is_null() {
                return (stride, &[]);
            }
            (stride, std::slice::from_raw_parts(data, len))
        }
    }

    fn as_ptr(&self) -> *const AVFrame {
        self.ptr
    }

    fn as_mut_ptr(&mut self) -> *mut AVFrame {
        self.ptr
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from av_frame_alloc and is freed once, here.
        unsafe { (self.api.av_frame_free)(&mut self.ptr) }
    }
}

/// A picture scaler (FFmpeg's swscale context), freed on drop.
pub(crate) struct Scaler {
    ptr: *mut SwsContext,
    api: &'static Api,
}

impl Scaler {
    pub(crate) fn new() -> Result<Self> {
        let api = api()?;
        // SAFETY: sws_alloc_context takes no arguments and returns a new context or null.
        let ptr = unsafe { (api.sws_alloc_context)() };
        if ptr.is_null() {
            bail!("sws_alloc_context failed");
        }
        Ok(Self { ptr, api })
    }

    /// Converts `src` into `dst` (its format and size). Returns FFmpeg's result: negative on failure.
    pub(crate) fn scale(&mut self, src: &Frame, dst: &mut Frame) -> c_int {
        // SAFETY: `ptr` is a live context; both frames are live, and `dst` has its format and buffers set.
        unsafe { (self.api.sws_scale_frame)(self.ptr, dst.as_mut_ptr(), src.as_ptr()) }
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from sws_alloc_context and is freed once, here.
        unsafe { (self.api.sws_free_context)(&mut self.ptr) }
    }
}
