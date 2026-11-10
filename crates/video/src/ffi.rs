//! FFmpeg, loaded at run time. The libraries are opened by name (or, on Windows, by full path in the folder that holds
//! them, with the search limited to that folder and the system's), and each function is looked up by name at first
//! use. Nothing is linked, so no search path (PATH, the working folder) decides which FFmpeg runs, and a missing one is
//! an error from [`api`], not a failure to start.
//!
//! The types and constants come from FFmpeg's headers (build.rs), so their layouts are FFmpeg's. The functions'
//! signatures are written here from the same headers; the calls go through the pointers in [`Api`].

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

use std::ffi::{CStr, CString, OsStr, c_char, c_int, c_void};
use std::io::{Read, Seek, SeekFrom};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::OnceLock;

use anyhow::{Result, bail};
use libloading::Library;

/// `AV_NOPTS_VALUE`: no timestamp. The headers define it as a macro, which bindgen doesn't emit.
pub(crate) const NOPTS_VALUE: i64 = i64::MIN;

/// The functions pg-video calls, from the libraries that hold them. The libraries stay open for the process's life:
/// the pointers point into them.
pub(crate) struct Api {
    _libs: Vec<Library>,
    pub(crate) av_log_set_level: unsafe extern "C" fn(c_int),
    pub(crate) av_dict_get: unsafe extern "C" fn(
        *const AVDictionary,
        *const c_char,
        *const AVDictionaryEntry,
        c_int,
    ) -> *mut AVDictionaryEntry,
    pub(crate) av_frame_alloc: unsafe extern "C" fn() -> *mut AVFrame,
    pub(crate) av_frame_free: unsafe extern "C" fn(*mut *mut AVFrame),
    pub(crate) av_frame_get_buffer: unsafe extern "C" fn(*mut AVFrame, c_int) -> c_int,
    pub(crate) av_strerror: unsafe extern "C" fn(c_int, *mut c_char, usize) -> c_int,
    pub(crate) avformat_open_input: unsafe extern "C" fn(
        *mut *mut AVFormatContext,
        *const c_char,
        *const AVInputFormat,
        *mut *mut AVDictionary,
    ) -> c_int,
    pub(crate) avformat_find_stream_info: unsafe extern "C" fn(*mut AVFormatContext, *mut *mut AVDictionary) -> c_int,
    pub(crate) avformat_close_input: unsafe extern "C" fn(*mut *mut AVFormatContext),
    pub(crate) av_read_frame: unsafe extern "C" fn(*mut AVFormatContext, *mut AVPacket) -> c_int,
    pub(crate) avformat_seek_file: unsafe extern "C" fn(*mut AVFormatContext, c_int, i64, i64, i64, c_int) -> c_int,
    pub(crate) av_find_best_stream:
        unsafe extern "C" fn(*mut AVFormatContext, AVMediaType, c_int, c_int, *mut *const AVCodec, c_int) -> c_int,
    pub(crate) avformat_alloc_context: unsafe extern "C" fn() -> *mut AVFormatContext,
    pub(crate) avio_alloc_context: unsafe extern "C" fn(
        *mut u8,
        c_int,
        c_int,
        *mut c_void,
        Option<ReadCallback>,
        Option<WriteCallback>,
        Option<SeekCallback>,
    ) -> *mut AVIOContext,
    pub(crate) avio_context_free: unsafe extern "C" fn(*mut *mut AVIOContext),
    pub(crate) av_malloc: unsafe extern "C" fn(usize) -> *mut c_void,
    pub(crate) av_free: unsafe extern "C" fn(*mut c_void),
    pub(crate) av_packet_alloc: unsafe extern "C" fn() -> *mut AVPacket,
    pub(crate) av_packet_free: unsafe extern "C" fn(*mut *mut AVPacket),
    pub(crate) av_packet_unref: unsafe extern "C" fn(*mut AVPacket),
    pub(crate) avcodec_find_decoder: unsafe extern "C" fn(AVCodecID) -> *const AVCodec,
    pub(crate) avcodec_alloc_context3: unsafe extern "C" fn(*const AVCodec) -> *mut AVCodecContext,
    pub(crate) avcodec_free_context: unsafe extern "C" fn(*mut *mut AVCodecContext),
    pub(crate) avcodec_parameters_to_context:
        unsafe extern "C" fn(*mut AVCodecContext, *const AVCodecParameters) -> c_int,
    pub(crate) avcodec_open2:
        unsafe extern "C" fn(*mut AVCodecContext, *const AVCodec, *mut *mut AVDictionary) -> c_int,
    pub(crate) avcodec_send_packet: unsafe extern "C" fn(*mut AVCodecContext, *const AVPacket) -> c_int,
    pub(crate) avcodec_receive_frame: unsafe extern "C" fn(*mut AVCodecContext, *mut AVFrame) -> c_int,
    pub(crate) avcodec_get_name: unsafe extern "C" fn(AVCodecID) -> *const c_char,
    pub(crate) sws_alloc_context: unsafe extern "C" fn() -> *mut SwsContext,
    pub(crate) sws_scale_frame: unsafe extern "C" fn(*mut SwsContext, *mut AVFrame, *const AVFrame) -> c_int,
    pub(crate) sws_free_context: unsafe extern "C" fn(*mut *mut SwsContext),
}

/// FFmpeg's read callback of a custom I/O context: the next bytes into `buf`, or a negative error.
type ReadCallback = unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int;
/// FFmpeg's write callback (not used: the input is read only; the context is made without one).
type WriteCallback = unsafe extern "C" fn(*mut c_void, *const u8, c_int) -> c_int;
/// FFmpeg's seek callback: the new position, or a negative error. `whence` may carry `AVSEEK_FORCE`.
type SeekCallback = unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64;

static API: OnceLock<Result<Api, String>> = OnceLock::new();

/// FFmpeg, loaded on first use. Its error says what is missing, and where it was looked for.
pub(crate) fn api() -> Result<&'static Api> {
    API.get_or_init(load).as_ref().map_err(|e| anyhow::anyhow!("{e}"))
}

/// The library names and major versions (from the headers), and the folders the Windows DLLs are looked for in: the
/// executable's own (a shipped app's), then the FFmpeg `bin` build.rs found (a build's, the tests').
fn load() -> Result<Api, String> {
    let avutil = Lib::open("avutil", generated_major_avutil())?;
    let avcodec = Lib::open("avcodec", generated_major_avcodec())?;
    let avformat = Lib::open("avformat", generated_major_avformat())?;
    let swscale = Lib::open("swscale", generated_major_swscale())?;
    let (u, c, f, s) = (&avutil.0, &avcodec.0, &avformat.0, &swscale.0);
    // SAFETY: every lookup below copies a function's pointer out of the library that exports it, and the signature
    // is the one in its header (the types are the generated ones, from the same headers). The library is kept open
    // in `_libs`, so the pointer stays valid for as long as the `Api` is.
    let api = unsafe {
        Api {
            av_log_set_level: sym(u, "av_log_set_level")?,
            av_dict_get: sym(u, "av_dict_get")?,
            av_frame_alloc: sym(u, "av_frame_alloc")?,
            av_frame_free: sym(u, "av_frame_free")?,
            av_frame_get_buffer: sym(u, "av_frame_get_buffer")?,
            av_strerror: sym(u, "av_strerror")?,
            avformat_open_input: sym(f, "avformat_open_input")?,
            avformat_find_stream_info: sym(f, "avformat_find_stream_info")?,
            avformat_close_input: sym(f, "avformat_close_input")?,
            av_read_frame: sym(f, "av_read_frame")?,
            avformat_seek_file: sym(f, "avformat_seek_file")?,
            av_find_best_stream: sym(f, "av_find_best_stream")?,
            avformat_alloc_context: sym(f, "avformat_alloc_context")?,
            avio_alloc_context: sym(f, "avio_alloc_context")?,
            avio_context_free: sym(f, "avio_context_free")?,
            av_malloc: sym(u, "av_malloc")?,
            av_free: sym(u, "av_free")?,
            av_packet_alloc: sym(c, "av_packet_alloc")?,
            av_packet_free: sym(c, "av_packet_free")?,
            av_packet_unref: sym(c, "av_packet_unref")?,
            avcodec_find_decoder: sym(c, "avcodec_find_decoder")?,
            avcodec_alloc_context3: sym(c, "avcodec_alloc_context3")?,
            avcodec_free_context: sym(c, "avcodec_free_context")?,
            avcodec_parameters_to_context: sym(c, "avcodec_parameters_to_context")?,
            avcodec_open2: sym(c, "avcodec_open2")?,
            avcodec_send_packet: sym(c, "avcodec_send_packet")?,
            avcodec_receive_frame: sym(c, "avcodec_receive_frame")?,
            avcodec_get_name: sym(c, "avcodec_get_name")?,
            sws_alloc_context: sym(s, "sws_alloc_context")?,
            sws_scale_frame: sym(s, "sws_scale_frame")?,
            sws_free_context: sym(s, "sws_free_context")?,
            _libs: vec![avutil.0, avcodec.0, avformat.0, swscale.0],
        }
    };
    Ok(api)
}

const fn generated_major_avutil() -> u32 {
    LIBAVUTIL_VERSION_MAJOR
}
const fn generated_major_avcodec() -> u32 {
    LIBAVCODEC_VERSION_MAJOR
}
const fn generated_major_avformat() -> u32 {
    LIBAVFORMAT_VERSION_MAJOR
}
const fn generated_major_swscale() -> u32 {
    LIBSWSCALE_VERSION_MAJOR
}

/// Looks up `name` in `lib` as a function pointer of type `F`.
///
/// # Safety
///
/// `F` must be the C signature of the function `name` (`lib` must keep it loaded for as long as `F` is used).
unsafe fn sym<F: Copy>(lib: &Library, name: &str) -> Result<F, String> {
    let cname = CString::new(name).map_err(|e| e.to_string())?;
    // SAFETY: the caller's contract: `F` matches the function, and `lib` outlives the pointer.
    unsafe { lib.get::<F>(cname.as_bytes_with_nul()).map(|s| *s) }.map_err(|e| format!("{name}: {e}"))
}

/// One of FFmpeg's libraries, open.
struct Lib(Library);

impl Lib {
    /// The library `stem` of major version `major`: its file, in the first of the folders that holds it (on Windows: the
    /// executable's, then the `bin` build.rs found), else found by the system's own rules.
    fn open(stem: &str, major: u32) -> Result<Self, String> {
        let file = pg_platform::dll::file_name(stem, major);
        let candidates: Vec<PathBuf> = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .into_iter()
            .chain(option_env!("PG_FFMPEG_BIN").map(PathBuf::from))
            .collect();
        let path = pg_platform::dll::resolve(&file, &candidates)?;
        // SAFETY: FFmpeg's own library, from the folder the search found (or the system's search); loading it runs its
        // initialization code, as FFmpeg's does.
        let lib = unsafe { pg_platform::dll::load(&path) }.map_err(|e| format!("loading {}: {e}", path.display()))?;
        Ok(Self(lib))
    }
}

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

/// The size of the buffer FFmpeg reads a file through (its own file protocol uses 32 KiB; a larger one saves calls).
const IO_BUFFER: usize = 256 * 1024;
/// `AVSEEK_SIZE` and `AVSEEK_FORCE`: macros in the headers, which bindgen doesn't emit (as `NOPTS_VALUE`).
const AVSEEK_SIZE: c_int = 0x10000;
const AVSEEK_FORCE: c_int = 0x20000;
const SEEK_SET: c_int = 0;
const SEEK_CUR: c_int = 1;
const SEEK_END: c_int = 2;
/// FFmpeg's error codes: `AVERROR_EOF` is `-FFERRTAG('E','O','F',' ')` (the tag's four bytes, little-endian, as one
/// number; a test checks it), and `AVERROR(EIO)` is the errno negated.
const AVERROR_EOF: c_int = -541_478_725;
const AVERROR_EIO: c_int = -5;

/// The file an [`Input`] reads, through FFmpeg's I/O callbacks: its bytes as the file system has them, whatever the
/// file's name is. Owned by the `Input` (the callbacks point at it).
struct Source {
    file: std::fs::File,
    len: u64,
}

/// FFmpeg's read callback: the next bytes of the file. Rust panics must not unwind into FFmpeg, so a panic is caught
/// and reported as an I/O error.
unsafe extern "C" fn read_source(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `opaque` is the `Source` that `Input::open` gave this context, alive until the context is freed;
        // `buf` is `size` bytes of FFmpeg's buffer, valid for this call.
        let source = unsafe { &mut *opaque.cast::<Source>() };
        // SAFETY: FFmpeg gives `size` bytes at `buf` for this call (none if the size is negative).
        let out = unsafe { std::slice::from_raw_parts_mut(buf, usize::try_from(size).unwrap_or(0)) };
        match source.file.read(out) {
            Ok(0) => AVERROR_EOF,
            Ok(n) => c_int::try_from(n).unwrap_or(AVERROR_EIO),
            Err(_) => AVERROR_EIO,
        }
    }))
    .unwrap_or(AVERROR_EIO)
}

/// FFmpeg's seek callback. `AVSEEK_SIZE` asks for the file's length; `AVSEEK_FORCE` is a hint the file protocol takes
/// and this one doesn't need, so it is masked off.
unsafe extern "C" fn seek_source(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in `read_source`.
        let source = unsafe { &mut *opaque.cast::<Source>() };
        let whence = whence & !AVSEEK_FORCE;
        if whence == AVSEEK_SIZE {
            return i64::try_from(source.len).unwrap_or(-1);
        }
        let from = match whence {
            SEEK_SET => match u64::try_from(offset) {
                Ok(pos) => SeekFrom::Start(pos),
                Err(_) => return -1,
            },
            SEEK_CUR => SeekFrom::Current(offset),
            SEEK_END => SeekFrom::End(offset),
            _ => return -1,
        };
        source.file.seek(from).ok().and_then(|pos| i64::try_from(pos).ok()).unwrap_or(-1)
    }))
    .unwrap_or(-1)
}

/// The name FFmpeg's format probe reads: `pg-video`, with the file's extension when it is ASCII. The name is never a
/// path (the file is read through the I/O), so the file's own name stays out of FFmpeg.
fn hint_name(path: &Path) -> CString {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()));
    let name = match ext {
        Some(ext) => format!("pg-video.{ext}"),
        None => "pg-video".to_string(),
    };
    CString::new(name).unwrap_or_default()
}

/// Frees a custom I/O context, its buffer and the source it read (FFmpeg doesn't free a custom context's I/O: it is
/// ours). FFmpeg may have replaced the buffer, and the context's field says which one it is.
fn free_io(api: &Api, mut io: *mut AVIOContext, source: *mut Source) {
    // SAFETY: `io` is the context `open` made, freed once here; its buffer is the one FFmpeg holds in the field.
    unsafe {
        (api.av_free)((*io).buffer.cast());
        (api.avio_context_free)(&mut io);
        // SAFETY: `source` came from `Box::into_raw` in `open`, and no callback runs after the context is freed.
        drop(Box::from_raw(source));
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
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_of_file_code_is_ffmpegs_tag() {
        assert_eq!(AVERROR_EOF, -c_int::from_le_bytes(*b"EOF "));
    }
}
