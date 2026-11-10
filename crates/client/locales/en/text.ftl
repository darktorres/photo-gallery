# The words the front end shows (pg-client::text). Each entry is one `text::Message`, by its id; a test checks that the
# two lists name the same entries. Values are passed as text, except `$count`, which chooses the plural form. Spacing
# that must survive (a leading run of spaces) is written as a quoted literal, because Fluent trims a pattern's ends.

# The facet labels
type-image = Image
type-animated = Animated
type-video = Video
shape-landscape = Landscape
shape-portrait = Portrait
shape-square = Square
status-imported = Imported
status-pending = Not imported yet
status-failed = Unreadable
border-none = None
border-black = Black
border-white = White
border-color = Colored
border-blur = Blurred bars
ext-none = (none)

# The filter line (`text::describe`)
filter-all = all files
filter-unrated = unrated
filter-with-duplicates = with duplicates
filter-without-duplicates = without duplicates
filter-taken = taken { $days }
filter-modified = modified { $days }
# A span of days in the filter line, each end a day (YYYY-MM-DD) or empty
filter-days = { $from } – { $to }
# Between the values of one facet in the filter line, and between the line's parts
filter-list-separator = {", "}
filter-part-separator = {" · "}
filter-range-mb = { $from }–{ $to } MB
filter-range-mp = { $from }–{ $to } MP
filter-range-seconds = { $from }–{ $to } s
filter-borders = borders: { $values }
filter-borders-marked = borders marked not a border
filter-borders-not-marked = borders not marked
filter-name = name has “{ $name }”

# The grid cell's badge
badge-video = ▶ { $time }
badge-apng = APNG

# The info line
info-not-imported = not imported
info-dims = { $width } × { $height }
info-size-mb = { $size } MB
info-size-kb = { $size } KB
info-kept = { "   " }rating kept in the catalog
info-line = { $path }   { $dims }   { $size }{ $kept }{ $borders }

# The borders of a file (`text::borders_info`)
class-black = black
class-white = white
class-color = colored
class-blur = blurred
borders-info = { "   " }borders { $names } (L { $left } T { $top } R { $right } B { $bottom } px){ $marked }
borders-marked = { " — marked not a border" }

# The status line of a pass (`text::pass_status`)
pass-scan = Scanning { $root }
# A scan's status line: its name (`pass-scan`) and the images found so far
pass-found = { $what } · { $count } images found
pass-import = Importing
pass-borders = Finding borders
pass-crop = Saving cropped copies
pass-thumbs = Thumbnails at { $height } px
pass-working = { $what } { $done } of { $total } · { $rate }/s · { $eta }{ $failed }
pass-failed = { " · " }{ $count } failed

# A time left
eta-seconds = { $seconds } s left
eta-minutes = { $minutes } min left
eta-hours = { $hours } h { $minutes } min left

# The write-back line
writing-changes = { $count ->
    [one] Writing { $text } change into files
   *[other] Writing { $text } changes into files
}
changes-unwritten = { $count ->
    [one] { $text } change not yet in the files: { $error }
   *[other] { $text } changes not yet in the files: { $error }
}
# The latest failure when nothing is left queued (the change it was about went)
write-queue-failed = A change couldn't be written into its file: { $error }
# A copy into a file that failed up to its retries after it may have started (`text::write_incomplete`)
write-incomplete = { $file } may be incomplete: copying its change into it failed ({ $error }). Its whole new content is in { $temp }, and the copy is tried again
# The writer died too often in a row and isn't started again until the app is (no arguments).
write-stopped = Writing changes into the files has stopped after repeated failures. The changes are kept, and are written when the app starts again
# Queued writes dropped because their files left the library: how many since the last note, and the kind of the latest
# (`rating` or `turns`, a selector)
write-dropped = { $count ->
    [one] { $kind ->
        [rating] A queued rating was dropped: its file is no longer in the library
       *[turns] A queued turn was dropped: its file is no longer in the library
    }
   *[other] { $kind ->
        [rating] { $text } queued changes were dropped, the latest a rating: their files are no longer in the library
       *[turns] { $text } queued changes were dropped, the latest a turn: their files are no longer in the library
    }
}

# The job's notices
unreadable-folders = { $root }: { $count } folders couldn't be read
scan-failed = { $root }: { $error }
root-refused = { $root } is not scanned, because { $reason }. Remove it from the library to stop this notice.
mount-refused = { $mount } is not scanned, because { $reason }. The files there stay as the library has them.
# Why the scanner refuses a folder (`pg_model::refusal::RefusedReason`), selected by its code: the `$reason` of
# `root-refused`, `mount-refused` and `folder-refused`. `$fs` is a file system's name, `$error` the system's error.
refused-reason = { $code ->
    [network-path] it is a network (UNC) path
    [unreadable-volume] its volume can't be read: { $error }
    [unreadable-folder] the folder can't be read: { $error }
    [unreadable-real-path] the folder's real path can't be read: { $error }
    [unreadable-real-volume] its real volume can't be read: { $error }
    [unreadable-file-system] its file system can't be read: { $error }
    [network-drive] its drive is a network drive
    [removable-drive] its drive is removable
    [other-drive] its drive is not a fixed local disk
    [link] it is a link (a junction or a symbolic link)
    [real-not-local] its real location is not on a local drive
    [real-on-other-volume] its real location is on another volume
    [not-ntfs] its file system is { $fs }, not NTFS
    [nfs] it is on an NFS mount
    [smb] it is on an SMB or CIFS share
    [fat] its file system is FAT, whose file numbers change when it is mounted again
    [exfat] its file system is exFAT, whose file numbers change when it is mounted again
    [pseudo] it is the kernel's { $fs } file system, not a disk
    [unsupported-system] scanning is supported on Windows and Linux only
   *[other] it can't be scanned for its files' identities
}
scan-note = { $root }: { $kind ->
    [duplicates] { $count } files were skipped as duplicates (another path to the same file)
    [reparse] { $count } links or placeholders were indexed without being followed
    [ambiguous] { $count } files matched more than one earlier row, so nothing was carried over
    [unpaired] { $count } missing files kept their ratings; no file was found for them
    [replaced] { $count } files were replaced by new files with the same name; their content was reset
    [rebound] { $count } files were found under a new identity and kept their ratings
    [volume-id] { $count } volumes have no file system UUID, so their files are told apart by a device number that a restart can renumber
   *[other] { $count } scan notes
}
round-panicked = internal error in the background job: { $message }
# A file a job panicked on (caught: the job went on with the next file), as a crop's or a write's failure
file-panicked = internal error on this file: { $message }
cache-full = The thumbnail cache is full: { $made } thumbnails at { $height } px weren't made

# A crop job's note: the saved copies, then the skipped files and the failures (`text::cropped`)
cropped-saved = { $count ->
    [one] Saved { $text } cropped copy in { $folder }
   *[other] Saved { $text } cropped copies in { $folder }
}
cropped-skipped = { $count ->
    [one] { $text } file had nothing to crop any more
   *[other] { $text } files had nothing to crop any more
}
cropped-failed = { $failed } failed, the first: { $error }

# The Yandex panel
yandex-captcha = Yandex asked for a captcha, which the app can't answer. Try again later.
yandex-not-still = Yandex search takes still images (JPEG, PNG, WebP, GIF, BMP or TIFF).
yandex-uploading = Uploading the photo…
yandex-other-sizes = Reading the other sizes…
yandex-similar = Reading similar images…
yandex-unreadable-photo = Can't read the photo: { $error }
yandex-similar-stopped = Similar images stopped after { $shown } results: { $reason }
yandex-similar-failed = Similar images failed: { $reason }
yandex-not-started = Couldn't start the search or the save: { $error }
folder-dialog-failed = Couldn't open the folder dialog: { $error }
folder-unreadable = Couldn't read the folder: { $error }
yandex-nothing-picked = Pick at least one image to save.
yandex-no-folder = No folder was chosen, so nothing was saved.

# The thumbnail cache, started over when it was opened (`text::thumb_cache_started_over`): one entry per reason. A
# damaged file's `$path` is where the old file is kept; for the others, it is the cache file, made anew.
thumb-cache-damaged = The thumbnail cache was damaged, so it was started over; its thumbnails are made again. The old file is kept as { $path }
thumb-cache-not-a-database = The thumbnail cache was not a database, so it was started over; its thumbnails are made again. The old file is kept as { $path }
thumb-cache-other-catalog = The thumbnail cache { $path } was made for another catalog, so it was started over; its thumbnails are made again
thumb-cache-other-build = The thumbnail cache { $path } was made by another version of the app, so it was started over; its thumbnails are made again

# The library panel (the desktop's `Library`): the status line while paused, the notice line with how many others came
# in, and what adding, removing and rescanning a folder say
work-paused = Background work paused
paused-pass = Paused · { $status }
notice-more = { $count ->
    [one] { $notice } (and { $text } more)
   *[other] { $notice } (and { $text } more)
}
folder-already-added = { $folder } is already in the library: rescanning it
# A folder the scanner refuses (`$reason`: not on a local NTFS disk, or on a network file system) is not added
folder-refused = { $folder } can't be added: { $reason }
folder-not-added = The folder couldn't be added: { $error }
folder-not-removed = The folder couldn't be removed from the library: { $error }
rescan-failed = The library couldn't be scanned again: { $error }
# The background work (the job and the writer) couldn't start: nothing is done until the app is started again
library-unavailable = The background work couldn't start, so nothing is scanned, imported or written into the files: { $error }
# The catalog's list of the library's folders couldn't be read
folders-unreadable = The library's folders couldn't be read: { $error }
round-failed = The background job failed: { $error }
crop-failed = Saving cropped copies failed: { $error }

# The Yandex panel's search and save lines
yandex-starting = Starting…
yandex-saving = { $count ->
    [one] Saving { $text } file…
   *[other] Saving { $text } files…
}
yandex-saving-file = Saving { $n } of { $of }…
yandex-saved = { $count ->
    [one] Saved { $text } file to { $folder }
   *[other] Saved { $text } files to { $folder }
}
yandex-saved-failed = Saved { $written }, { $failed } failed. First failure: { $error }
yandex-download-failed = { $url } couldn't be downloaded: { $error }
yandex-write-failed = { $url } couldn't be written to { $path }: { $error }
# A search or a save stopped by a panic, which the app logged
yandex-stopped = Stopped by an internal error; the details are in gallery.log.
# Why a Yandex request failed (`pg_yandex::YandexError`; the captcha is `yandex-captcha`)
yandex-http-status = Yandex answered HTTP { $status }
yandex-transport = Yandex request failed: { $error }
yandex-unexpected = unexpected Yandex response: { $detail }
yandex-recorded = recorded Yandex session: { $error }
yandex-undecodable = a result's file is not an image this app can decode: { $reason }

# The boxes the app shows before it ends: the catalog that can't be opened, and a panic (`text::internal_error`: the
# first line, the panic's message, then where its details are)
catalog-unopened = Can't open the catalog: { $error }
internal-error = Photo Gallery hit an internal error:
details-in-log = The details are in { $log }.
details-not-logged = Writing them to { $log } failed ({ $error }).
