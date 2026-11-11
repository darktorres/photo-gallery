//! The images the front end shows: thumbnails, placeholders, and the original as the viewer asks for it. Each image is made
//! from one consistent view of its file, never a mix: a thumbnail the cache keeps for the file as its key names it is
//! served from the key alone, turned by the key's turns; anything else is made from the file as the catalog has it now,
//! turned by the catalog's turns, and checked against the catalog before and after the file is read (`consistent`), so
//! that a write of the app's into the file at that moment is neither shown twice nor kept under the state before it.
//!
//! A thumbnail made on a miss goes to the cache's writer once it is shown, moved there rather than copied ([`Keep`]):
//! the front end hands it back with [`Images::keep`] once it is done with the bytes.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pg_catalog::ledger::Failure;
use pg_model::ids::FileId;
use pg_model::image::{ImageKey, Version, View};
use pg_model::state::FileSource;
use pg_model::turns::Turns;
use pg_render::display::{self, Rgba, Shown};
use pg_render::maker::MakeError;
use pg_thumbcache::{self as thumbs, ThumbCache};

use crate::catalogs::{Catalogs, path_of};
use crate::error::{Error, Result, failed, made_error, original};
use crate::inbox::Inbox;
use crate::makers::Makers;

/// How long a request waits for a write of the app's into its file to end before it fails.
const WRITE_WAIT: Duration = Duration::from_secs(10);

/// How many of the thumbnails that failed lately are remembered, so one asked for again (at every scroll past it) is not
/// sent to the ledger again.
const RECENT_FAILURES: usize = 256;

/// The thumbnail cache's source of a key's version: the file as the grid showed it when the key was made.
fn source_of(v: Version) -> thumbs::Source {
    thumbs::Source { mtime: v.mtime, rotation: v.rotation }
}

/// What a thumbnail request found, as the catalog has the file.
enum Found {
    /// In the cache at this height: read into the caller's buffer.
    Kept(u32),
    /// Made from the original.
    Made(Vec<u8>),
    /// The original can't be read (offline), or doesn't decode. The height of the best smaller thumbnail the cache keeps
    /// of the file as it is, if it was read into the caller's buffer.
    Offline(Error, Option<u32>),
}

/// A thumbnail, as [`Images::thumbnail`] finds it.
pub struct Thumbnail {
    /// Quarter turns clockwise to show it turned by.
    pub turns: Turns,
    /// One made from the original: its bytes, and where the cache keeps it once it is shown. Otherwise the bytes are the
    /// cache's, in the buffer [`Images::thumbnail`] was given.
    pub made: Option<(Vec<u8>, Keep)>,
    /// For a cache's: the height it is kept at, to drop it ([`Images::forget_thumb`]) when it doesn't decode.
    pub kept_at: Option<u32>,
}

/// Where the cache keeps a thumbnail made on a miss ([`Images::keep`]).
#[derive(Clone, Copy, Debug)]
pub struct Keep {
    pub file_id: FileId,
    pub height: u32,
    pub source: thumbs::Source,
}

/// The last full-size decode, kept for the zoom regions that follow each other while the user zooms and pans one image.
/// It is kept for the app, until the next decode: a 24 MP image is about 72 MB of RGB.
pub(crate) struct Decoded {
    file_id: FileId,
    mtime: i64,
    /// The file's size: a decode is reused only while the file still has it.
    size: i64,
    /// With the turns not yet written.
    rotation: Turns,
    /// Upright, possibly DCT-scaled, with its full size.
    image: Arc<Shown>,
}

/// The images the front end shows: thumbnails, placeholders, and the viewer's original, as the catalog, the thumbnail cache
/// and the files have them.
pub struct Images {
    catalogs: Arc<Catalogs>,
    makers: Arc<Makers>,
    /// The thumbnail cache, leaked to a `'static` reference so the library's threads can borrow it. Closed at shutdown.
    thumbs: &'static ThumbCache,
    /// The last full-size decode (`Images::full_image`).
    last_decode: Mutex<Option<Decoded>>,
    /// Where a thumbnail that doesn't decode is recorded (the thumbnails' failure ledger), and one made leaves it.
    inbox: Arc<Inbox>,
    /// The files whose thumbnail failed lately, as they were (id, mtime, size, turns): sent to the ledger once.
    recent: Mutex<VecDeque<(i64, i64, i64, u8)>>,
}

impl Images {
    pub(crate) fn new(
        catalogs: Arc<Catalogs>,
        makers: Arc<Makers>,
        thumbs: &'static ThumbCache,
        inbox: Arc<Inbox>,
    ) -> Images {
        Images { catalogs, makers, thumbs, last_decode: Mutex::new(None), inbox, recent: Mutex::new(VecDeque::new()) }
    }

    /// A thumbnail of the file as `src` has it that doesn't decode: recorded in the thumbnails' ledger (once while it is
    /// recent), and what the request found.
    fn undecodable(&self, file_id: FileId, src: &FileSource, e: MakeError, kept: Option<u32>) -> Found {
        let error = made_error(e);
        if let Error::Undecodable(text) = &error {
            let state = &src.state;
            let key = (file_id.get(), state.mtime, state.size, state.rotation.get());
            let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
            if !recent.contains(&key) {
                if recent.len() >= RECENT_FAILURES {
                    recent.pop_front();
                }
                recent.push_back(key);
                drop(recent);
                pg_log::warn("thumbs", &format!("file {}: {text}", file_id.get()));
                self.inbox.thumb_failed(Failure {
                    file_id,
                    mtime: state.mtime,
                    size: state.size,
                    rotation: state.rotation,
                    error: text.clone(),
                    retry: true,
                });
            }
        }
        Found::Offline(error, kept)
    }

    /// `read` of the file, consistent with the catalog: the file's state is read before and after it, and the result
    /// counts only when no write of the app's into the file was in progress and the state stayed the same. The writer
    /// marks a write before it rewrites the file and records it in the transaction that ends the mark, so a write that
    /// overlapped the read shows in one state or the other. Otherwise it is read again once the write is done.
    fn consistent<T>(
        &self,
        file_id: FileId,
        mut read: impl FnMut(&FileSource) -> Result<T>,
    ) -> Result<(FileSource, T)> {
        let start = Instant::now();
        loop {
            let src = self.source(file_id)?;
            if !src.state.writing {
                let result = read(&src);
                if self.catalogs.with_catalog(|c| c.file_state(file_id))? == Some(src.state) {
                    return result.map(|r| (src, r));
                }
            }
            if start.elapsed() > WRITE_WAIT {
                return Err(Error::Rewriting(file_id));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn source(&self, file_id: FileId) -> Result<FileSource> {
        self.catalogs.with_catalog(|c| c.file_source(file_id))?.ok_or(Error::UnknownFile(file_id))
    }

    /// A JPEG thumbnail `height` px high, and the quarter turns clockwise to show it turned by. When the cache keeps one at
    /// least that high for the file as the key's version names it, that one, turned by the version's turns: nothing else
    /// is read. Otherwise as the catalog has the file now: kept, or made from the original (and to be kept), and turned by
    /// the catalog's turns. With the original offline, the best smaller one.
    ///
    /// A kept one is read into `jpeg`, a buffer the caller keeps from one thumbnail to the next, so that showing a cached
    /// thumbnail makes no buffer for its bytes.
    pub fn thumbnail(&self, key: ImageKey, height: u32, jpeg: &mut Vec<u8>) -> Result<Thumbnail> {
        let (file_id, named) = (key.file, key.version);
        let kept = |turns, at| Thumbnail { turns, made: None, kept_at: Some(at) };
        // The height of a smaller one of the file as named, read into `jpeg`.
        let mut smaller = None;
        if let Some(n) = named {
            match self.thumbs.get_nearest_into(file_id, height, source_of(n), jpeg).map_err(failed)? {
                Some(h) if h >= height => return Ok(kept(n.turns, h)),
                Some(h) => smaller = Some(h),
                None => {}
            }
        }
        let (src, found) = self.consistent(file_id, |src| {
            let current = thumbs::Source { mtime: src.state.mtime, rotation: src.state.rotation };
            let kept_smaller = if named.is_some_and(|n| source_of(n) == current) {
                // Looked for already.
                None
            } else {
                match self.thumbs.get_nearest_into(file_id, height, current, jpeg).map_err(failed)? {
                    Some(h) if h >= height => return Ok(Found::Kept(h)),
                    found => {
                        // A read overwrote the named one's bytes.
                        if found.is_some() {
                            smaller = None;
                        }
                        found
                    }
                }
            };
            // The file's disk path, which std and FFmpeg reach exactly.
            let path = path_of(&src.path);
            // The maker's failures are the original's (gone, offline, not decodable), so the variant is made here:
            // `with_maker` would make any error its closure returns an `Error::Failed`.
            let made = if let Some(at) = src.poster_ms {
                // A video: its poster frame.
                self.makers.with_maker(|m| {
                    Ok(m.load_frame(&path, at, current.rotation)
                        .and_then(|()| m.encode_frame(pg_model::num::index(height))))
                })?
            } else {
                // Read into the maker, which keeps the buffer for the next file.
                match self.makers.with_maker(|m| Ok(m.read_file(&path)))? {
                    Ok(()) => {}
                    Err(e) => return Ok(Found::Offline(original(e), kept_smaller)),
                }
                self.makers.with_maker(|m| Ok(m.make_from_file(pg_model::num::index(height), current.rotation)))?
            };
            Ok(match made {
                Ok(made) => Found::Made(made),
                Err(MakeError::Unreadable(e)) => Found::Offline(original(e), kept_smaller),
                Err(e @ MakeError::Undecodable(_)) => self.undecodable(file_id, src, e, kept_smaller),
            })
        })?;
        let turns = src.state.turns;
        match found {
            Found::Kept(h) => Ok(kept(turns, h)),
            Found::Offline(_, Some(h)) => Ok(kept(turns, h)),
            Found::Made(made) => {
                // A file whose thumbnail failed (in a pass, or here) and is made now leaves the ledger.
                self.inbox.thumb_made(file_id);
                let source = thumbs::Source { mtime: src.state.mtime, rotation: src.state.rotation };
                Ok(Thumbnail { turns, made: Some((made, Keep { file_id, height, source })), kept_at: None })
            }
            Found::Offline(e, None) => {
                let Some(n) = named else { return Err(e) };
                let at = match smaller {
                    Some(h) => Some(h),
                    // A read of the file as it is (an earlier try of `consistent`) overwrote it: read again.
                    None => self.thumbs.get_nearest_into(file_id, height, source_of(n), jpeg).map_err(failed)?,
                };
                match at {
                    Some(h) => Ok(kept(n.turns, h)),
                    None => Err(e),
                }
            }
        }
    }

    /// Drop the cache's thumbnail of `file_id` at `height`, one that doesn't decode: the next pass at that height makes
    /// it anew from the original ([`thumbs::ThumbCache::forget_height`]).
    pub fn forget_thumb(&self, file_id: FileId, height: u32) -> Result<()> {
        self.thumbs.forget_height(file_id, height).map_err(failed)
    }

    /// Put a thumbnail made on a miss into the cache, in the background: it is shown whatever the write does.
    pub fn keep(&self, keep: Keep, data: Vec<u8>) {
        self.thumbs.put(thumbs::Thumb { file_id: keep.file_id, height: keep.height, source: keep.source, data });
    }

    /// A thumbnail (a JPEG) turned by `turns` quarter turns clockwise, for a turn not yet written into the file.
    pub fn turn_jpeg(&self, jpeg: &[u8], turns: Turns) -> Result<Rgba> {
        self.makers.with_maker(|m| display::decode_full(m.decoder(), jpeg, turns)?.rgba(View::Full))
    }

    /// The ThumbHash placeholder as a tiny image, shown as the catalog has the file now.
    pub fn placeholder(&self, file_id: FileId) -> Result<Rgba> {
        let p = self.catalogs.with_catalog(|c| c.placeholder(file_id))?.ok_or(Error::UnknownFile(file_id))?;
        let hash = p.hash.ok_or(Error::NotImported)?;
        display::placeholder(hash.as_slice(), p.turns).map_err(failed)
    }

    /// The original as the catalog has it now, upright (EXIF and catalog rotation, then the turns not yet written), as the
    /// viewer asks for it.
    pub fn full_image(&self, file_id: FileId, view: View) -> Result<Rgba> {
        let (_, (img, fresh)) = self.consistent(file_id, |src| self.decode(file_id, src, view))?;
        // Kept once it is known to be of the file as the catalog has it.
        if let Some(fresh) = fresh {
            *self.last_decode.lock().unwrap_or_else(PoisonError::into_inner) = Some(fresh);
        }
        // The decode stays shared (`last_decode`); what is shown is made from it.
        img.rgba(view).map_err(failed)
    }

    /// Decode what `view` needs (`display::min_size`), turned as the catalog says, reusing the last decode of the file in
    /// that state when it is big enough; a new decode comes with what to keep of it.
    fn decode(&self, file_id: FileId, src: &FileSource, view: View) -> Result<(Arc<Shown>, Option<Decoded>)> {
        let rotation = src.state.rotation.plus(src.state.turns);
        {
            let last = self.last_decode.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(d) = last.as_ref()
                && d.file_id == file_id
                && d.mtime == src.state.mtime
                && d.size == src.state.size
                && d.rotation == rotation
                && d.image.covers(view)
            {
                return Ok((d.image.clone(), None));
            }
        }
        let image = if let Some(at) = src.poster_ms {
            // A video: its poster frame stands for it until it plays.
            Arc::new(display::poster(&path_of(&src.path), at, rotation).map_err(original)?)
        } else {
            // The read's and the decode's failures are the original's (gone, offline, not decodable), so the variant is
            // made here: `with_maker` would make any error its closure returns an `Error::Failed`. A maker that can't
            // be made is still that.
            let path = path_of(&src.path);
            let decoded = self.makers.with_maker(|m| {
                Ok(match m.read_file(&path) {
                    Ok(()) => m.decode_file(view, rotation).map_err(made_error),
                    Err(e) => Err(original(e)),
                })
            })??;
            Arc::new(decoded)
        };
        Ok((image.clone(), Some(Decoded { file_id, mtime: src.state.mtime, size: src.state.size, rotation, image })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use pg_catalog::fixture::{self, File};
    use pg_model::ids::FolderId;

    // A file the catalog holds whose original is gone (deleted, or on a drive that is offline) is `Error::Original`,
    // which a caller can branch on, for the viewer's original and for a thumbnail alike; its text says so once.
    #[test]
    fn an_original_that_is_gone_is_error_original() {
        let dir = pg_testkit::test_folder("scratch_app", "original-gone").unwrap();
        let app = App::open(Config::in_home(&dir)).unwrap();
        let folder = if cfg!(windows) { r"D:\pg-test-no-such-folder" } else { "/pg-test-no-such-folder" };
        app.catalogs()
            .with_catalog(|c| {
                fixture::state::add_scan(c, 1, folder).unwrap();
                fixture::state::add_folder(c, 1, folder).unwrap();
                fixture::insert(c, &[File::new(FileId::new(1), FolderId::new(1), "gone.jpg", 10, 0)]).unwrap();
                Ok(())
            })
            .unwrap();
        match app.images().full_image(FileId::new(1), View::Full) {
            Err(e @ Error::Original(_)) => {
                let text = e.to_string();
                assert!(text.starts_with("original unavailable: "), "{text}");
                assert_eq!(text.matches("original unavailable").count(), 1, "{text}");
            }
            Err(e) => panic!("not Error::Original: {e:?}"),
            Ok(_) => panic!("an image of a file that isn't there"),
        }
        let key = ImageKey { file: FileId::new(1), version: None };
        match app.images().thumbnail(key, 100, &mut Vec::new()) {
            Err(Error::Original(_)) => {}
            Err(e) => panic!("not Error::Original: {e:?}"),
            Ok(_) => panic!("a thumbnail of a file that isn't there"),
        }
        app.shutdown();
        drop(app);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!dir.exists(), "nothing holds the app's files after the shutdown");
    }

    /// The thumbnails' ledger, once the inbox's writer holds `want` files (it writes in the background).
    fn ledger_until(app: &App, want: usize) -> Vec<FileId> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let held: Vec<FileId> = app
                .catalogs()
                .with_catalog(|c| c.recorded_failures(pg_model::notice::FilePass::Thumbs))
                .unwrap()
                .into_iter()
                .collect();
            if held.len() == want || Instant::now() > deadline {
                return held;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    // A still image that doesn't decode is `Error::Undecodable` (not an unavailable original), and its thumbnail's failure
    // is recorded in the ledger; once the file decodes, a thumbnail made of it takes it out.
    #[test]
    fn a_thumbnail_that_doesnt_decode_is_recorded_until_one_is_made() {
        let dir = pg_testkit::test_folder("scratch_app", "thumb-undecodable").unwrap();
        let images = dir.join("images");
        std::fs::create_dir_all(&images).unwrap();
        let file = images.join("bad.png");
        std::fs::write(&file, b"not a picture").unwrap();
        let app = App::open(Config::in_home(&dir)).unwrap();
        let folder = images.to_str().unwrap();
        app.catalogs()
            .with_catalog(|c| {
                fixture::state::add_scan(c, 1, folder).unwrap();
                fixture::state::add_folder(c, 1, folder).unwrap();
                fixture::insert(c, &[File::new(FileId::new(1), FolderId::new(1), "bad.png", 10, 0)]).unwrap();
                Ok(())
            })
            .unwrap();
        let key = ImageKey { file: FileId::new(1), version: None };
        for _ in 0..2 {
            match app.images().thumbnail(key, 100, &mut Vec::new()) {
                Err(Error::Undecodable(_)) => {}
                Err(e) => panic!("not Error::Undecodable: {e:?}"),
                Ok(_) => panic!("a thumbnail of a file that doesn't decode"),
            }
        }
        assert_eq!(ledger_until(&app, 1), [FileId::new(1)]);
        assert_eq!(app.images().recent.lock().unwrap().len(), 1, "asked twice, sent once");
        std::fs::write(&file, pg_testkit::png_rgb(8, 8, &[128; 192]).unwrap()).unwrap();
        assert!(app.images().thumbnail(key, 100, &mut Vec::new()).unwrap().made.is_some());
        assert!(ledger_until(&app, 0).is_empty());
        app.stop().unwrap();
        app.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An app on a test folder whose catalog holds file `id`, `name` in the folder `images` (made), size 10 and mtime 0.
    fn app_with(name: &str, id: i64, file: &'static str) -> (App, std::path::PathBuf, std::path::PathBuf) {
        let dir = pg_testkit::test_folder("scratch_app", name).unwrap();
        let images = dir.join("images");
        std::fs::create_dir_all(&images).unwrap();
        let app = App::open(Config::in_home(&dir)).unwrap();
        let folder = images.to_str().unwrap();
        app.catalogs()
            .with_catalog(|c| {
                fixture::state::add_scan(c, 1, folder).unwrap();
                fixture::state::add_folder(c, 1, folder).unwrap();
                fixture::insert(c, &[File::new(FileId::new(id), FolderId::new(1), file, 10, 0)]).unwrap();
                Ok(())
            })
            .unwrap();
        (app, images, dir)
    }

    /// The thumbnails' ledger once the inbox's writer has written what was sent to it.
    fn ledger_now(app: &App) -> Vec<FileId> {
        assert!(app.inbox().settle());
        app.catalogs()
            .with_catalog(|c| c.recorded_failures(pg_model::notice::FilePass::Thumbs))
            .unwrap()
            .into_iter()
            .collect()
    }

    // Only a thumbnail that doesn't decode is recorded: an original that can't be read, a file being rewritten and a
    // file the catalog doesn't have are not (none of them says anything about the file's bytes).
    #[test]
    fn only_an_undecodable_thumbnail_is_recorded() {
        let (app, images, dir) = app_with("thumb-not-recorded", 1, "gone.jpg");
        let thumb = |id| {
            let key = ImageKey { file: FileId::new(id), version: None };
            app.images().thumbnail(key, 100, &mut Vec::new())
        };
        assert!(matches!(thumb(1), Err(Error::Original(_))));
        assert!(matches!(thumb(2), Err(Error::UnknownFile(_))));
        // A write of the app's whose copy is in progress (its temp recorded): the request waits, then gives up.
        std::fs::write(images.join("gone.jpg"), b"not a picture").unwrap();
        let temp = pg_platform::native::PATH_RULES.parse(images.join("gone.jpg.tmp").to_str().unwrap()).unwrap();
        app.catalogs()
            .with_catalog(|c| {
                fixture::state::queue_finished_copy(c, FileId::new(1), "rating", 3, &temp).unwrap();
                Ok(())
            })
            .unwrap();
        assert!(matches!(thumb(1), Err(Error::Rewriting(_))));
        assert!(app.images().recent.lock().unwrap().is_empty());
        assert!(ledger_now(&app).is_empty());
        app.stop().unwrap();
        app.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A thumbnail that doesn't decode is recorded even when a smaller one the cache keeps is shown instead (the
    // record comes before the fallback), and it is logged once however often it is asked for.
    #[test]
    fn a_failure_is_recorded_before_the_smaller_fallback_and_logged_once() {
        const ID: i64 = 7301;
        let (app, images, dir) = app_with("thumb-fallback", ID, "bad.png");
        std::fs::write(images.join("bad.png"), b"not a picture").unwrap();
        let file = FileId::new(ID);
        let source = thumbs::Source { mtime: 0, rotation: Turns::NONE };
        app.images().keep(Keep { file_id: file, height: 50, source }, b"a smaller thumbnail".to_vec());
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.images().thumbs.get_nearest_into(file, 50, source, &mut Vec::new()).unwrap().is_none() {
            assert!(Instant::now() < deadline, "the cache keeps the smaller thumbnail");
            std::thread::sleep(Duration::from_millis(20));
        }
        let feed = pg_log::subscribe();
        for _ in 0..3 {
            let mut jpeg = Vec::new();
            let shown = app.images().thumbnail(ImageKey { file, version: None }, 100, &mut jpeg).unwrap();
            assert_eq!((shown.kept_at, jpeg.as_slice()), (Some(50), &b"a smaller thumbnail"[..]));
        }
        assert_eq!(ledger_now(&app), [file]);
        let logged =
            feed.new.try_iter().filter(|r| r.target == "thumbs" && r.message.starts_with("file 7301:")).count();
        assert_eq!(logged, 1);
        app.stop().unwrap();
        app.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The viewer's original: bytes that don't decode are `Undecodable`, an original that can't be opened is `Original`.
    #[test]
    fn the_full_image_tells_undecodable_from_unavailable() {
        let (app, images, dir) = app_with("full-undecodable", 1, "bad.jpg");
        assert!(matches!(app.images().full_image(FileId::new(1), View::Full), Err(Error::Original(_))));
        // A JPEG's start, then nothing it can decode.
        std::fs::write(images.join("bad.jpg"), b"\xFF\xD8\xFF\xE0 cut short").unwrap();
        assert!(matches!(app.images().full_image(FileId::new(1), View::Full), Err(Error::Undecodable(_))));
        app.stop().unwrap();
        app.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
