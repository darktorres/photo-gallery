//! The loader: FFmpeg's libraries, opened by name (or, on Windows, by full path in the folder that holds them), the
//! functions the crate calls looked up in them ([`Api`]), and the major versions checked against the headers'.

use std::ffi::{CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;
use libloading::Library;

use super::generated::*;

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
