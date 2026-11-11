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
pass-working = { $what } { $done } of { $total } · { $rate }/s · { $eta }
# The status bar's link to the pass's failed files, beside its line
pass-failed = { $count } failed

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

# Why the scanner refuses a folder (`pg_model::refusal::RefusedReason`), selected by its code: the `$reason` of
# the notices `root-refused`, `mount-refused` and `folder-refused`. `$file_system` is a file system's name, `$error` the system's error.
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
    [not-ntfs] its file system is { $file_system }, not NTFS
    [nfs] it is on an NFS mount
    [smb] it is on an SMB or CIFS share
    [fat] its file system is FAT, whose file numbers change when it is mounted again
    [exfat] its file system is exFAT, whose file numbers change when it is mounted again
    [pseudo] it is the kernel's { $file_system } file system, not a disk
    [unsupported-system] scanning is supported on Windows and Linux only
   *[other] it can't be scanned for its files' identities
}
# A file a job panicked on (caught: the job went on with the next file), as a crop's or a write's failure
file-panicked = internal error on this file: { $message }

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
# A path given to add (a folder, or a file named on the command line) that can't be opened, or isn't one
path-unreadable = Can't open { $path }: { $error }
not-a-path = { $path } is not a path
yandex-nothing-picked = Pick at least one image to save.
yandex-no-folder = No folder was chosen, so nothing was saved.

# The library panel (the desktop's `Library`): the status line while paused
work-paused = Background work paused
paused-pass = Paused · { $status }

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

# The notices (`pg_model::notice::Notice`): each kind's title (the inbox row, the toast) and body (the details), by its
# code. `$count` chooses a plural form and `$text` shows it; a path is as the system writes it.
notice-scan-unreadable-title = Folders couldn't be read
notice-scan-unreadable-body = { $count ->
    [one] { $root }: { $text } folder couldn't be read, so its files weren't scanned
   *[other] { $root }: { $text } folders couldn't be read, so their files weren't scanned
}
notice-scan-failed-title = A scan failed
notice-scan-failed-body = { $root }: { $error }
notice-root-refused-title = A folder isn't scanned
notice-root-refused-body = { $root } is not scanned, because { $reason }. Remove it from the library to stop this notice.
notice-mount-refused-title = A folder inside the library isn't scanned
notice-mount-refused-body = { $mount } is not scanned, because { $reason }. The files there stay as the library has them.
notice-scan-note-title = { $kind ->
    [duplicates] Duplicate paths skipped
    [reparse] Links indexed without being followed
    [ambiguous] Files that couldn't be matched
    [unpaired] Missing files kept
    [replaced] Files replaced
    [rebound] Files found again
    [volume-id] Volumes without a UUID
   *[other] A scan note
}
notice-scan-note-body = { $root }: { $kind ->
    [duplicates] { $text } files were skipped as duplicates (another path to the same file)
    [reparse] { $text } links or placeholders were indexed without being followed
    [ambiguous] { $text } files matched more than one earlier row, so nothing was carried over
    [unpaired] { $text } missing files kept their ratings; no file was found for them
    [replaced] { $text } files were replaced by new files with the same name; their content was reset
    [rebound] { $text } files were found under a new identity and kept their ratings
    [volume-id] { $text } volumes have no file system UUID, so their files are told apart by a device number that a restart can renumber
   *[other] { $text } scan notes
}
notice-round-failed-title = The background job failed
notice-round-failed-body = { $error }
notice-round-panicked-title = Internal error in the background job
notice-round-panicked-body = { $message }. The job went on with the next request; the details are in the log.
notice-cache-full-title = The thumbnail cache is full
notice-cache-full-body = { $made } thumbnails at { $height } px weren't made. Make the cache bigger, or let it drop old thumbnails.
notice-cropped-title = Cropped copies saved
notice-cropped-body = { $saved } saved in { $folder }; { $skipped } had nothing to crop; { $failed ->
    [0] none failed
   *[other] { $failed } failed, the first: { $path }: { $error }
}
notice-crop-failed-title = Saving cropped copies failed
notice-crop-failed-body = { $error }
notice-catalog-rebuilt-title = The catalog was rebuilt
notice-catalog-rebuilt-body = It didn't match this version of the app ({ $why }), so a new one was made and the old one kept as { $kept }. { $carried ->
    [rows] It took the old one's { $files } files and { $folders } library folders.
   *[nothing] It took nothing from the old one ({ $reason }): the next scan fills it.
}
notice-catalog-not-closed-title = The catalog wasn't closed
notice-catalog-not-closed-body = { $error }
notice-prefs-unreadable-title = The preferences couldn't be read
notice-prefs-unreadable-body = { $path } didn't parse ({ $error }), so the defaults are in use. { $kept ->
    [none] It couldn't be kept aside: { $keep_error }
   *[other] It was kept as { $kept }.
}
notice-prefs-not-saved-title = The preferences couldn't be saved
notice-prefs-not-saved-body = { $path }: { $error }. The change holds until the app closes.
notice-cache-totals-fixed-title = The thumbnail cache's totals were fixed
notice-cache-totals-fixed-body = They said { $kept_count } thumbnails in { $kept_bytes } bytes; the cache holds { $count } in { $bytes } bytes.
notice-cache-touches-not-written-title = The thumbnail cache couldn't note what was shown
notice-cache-touches-not-written-body = { $error }. It tries again later.
notice-cache-not-stored-title = A thumbnail couldn't be kept
notice-cache-not-stored-body = { $error }. It was shown all the same.
notice-cache-wal-not-merged-title = The thumbnail cache wasn't closed cleanly
notice-cache-wal-not-merged-body = { $error }. It is merged when the app starts again.
notice-cache-started-over-title = The thumbnail cache was started over
notice-cache-started-over-body = { $reason ->
    [damaged] The cache was damaged, so it was started over; its thumbnails are made again. The old file is kept as { $path }
    [not-a-database] The cache was not a database, so it was started over; its thumbnails are made again. The old file is kept as { $path }
    [other-catalog] The cache { $path } was made for another catalog, so it was started over; its thumbnails are made again
   *[other-build] The cache { $path } was made by another version of the app, so it was started over; its thumbnails are made again
}
notice-cache-disabled-title = Thumbnails aren't kept
notice-cache-disabled-body = The thumbnail cache couldn't be opened, so each thumbnail is made when it is shown: { $error }
notice-write-failed-title = A change couldn't be written into its file
notice-write-failed-body = { $file ->
    [none] { $error }. It is tried again later.
   *[other] { $file } may be incomplete: copying its change into it failed ({ $error }). Its whole new content is in { $temp }, and the copy is tried again.
}
notice-write-stopped-title = Writing into the files has stopped
notice-write-stopped-body = Writing changes into the files has stopped after repeated failures. The changes are kept, and are written when the app starts again.
notice-write-dropped-title = Queued changes were dropped
notice-write-dropped-body = { $count ->
    [one] { $kind ->
        [rating] A queued rating was dropped: its file is no longer in the library
       *[turns] A queued turn was dropped: its file is no longer in the library
    }
   *[other] { $kind ->
        [rating] { $text } queued changes were dropped, the latest a rating: their files are no longer in the library
       *[turns] { $text } queued changes were dropped, the latest a turn: their files are no longer in the library
    }
}
notice-folder-already-added-title = { $folder } is already in the library
notice-folder-already-added-body = It is scanned again.
notice-folder-refused-title = The folder can't be added
notice-folder-refused-body = { $folder } can't be added: { $reason }
notice-folder-not-added-title = The folder couldn't be added
notice-folder-not-added-body = { $error }
notice-folder-not-removed-title = The folder couldn't be removed from the library
notice-folder-not-removed-body = { $error }
notice-rescan-failed-title = The library couldn't be scanned again
notice-rescan-failed-body = { $error }
notice-library-unavailable-title = The background work couldn't start
notice-library-unavailable-body = Nothing is scanned, imported or written into the files: { $error }
notice-folders-unreadable-title = The library's folders couldn't be read
notice-folders-unreadable-body = { $error }
notice-picker-failed-title = The folder dialog failed
notice-picker-failed-body = { $error }
notice-listing-failed-title = The files couldn't be listed
notice-listing-failed-body = { $error }
notice-row-read-failed-title = A file's details couldn't be read
notice-row-read-failed-body = { $error }
notice-reload-failed-title = A changed file couldn't be read again
notice-reload-failed-body = { $error }
notice-facets-failed-title = The filters' counts couldn't be read
notice-facets-failed-body = { $error }
notice-tree-failed-title = The folder tree couldn't be read
notice-tree-failed-body = { $error }
notice-views-failed-title = { $op ->
    [list] The saved views couldn't be listed
    [save] The view couldn't be saved
   *[delete] The view couldn't be deleted
}
notice-views-failed-body = { $error }
notice-session-not-saved-title = Where you were couldn't be saved
notice-session-not-saved-body = { $error }
notice-edit-failed-title = { $op ->
    [rate] A rating couldn't be saved
    [rotate] A turn couldn't be saved
   *[mark-border] A border mark couldn't be saved
}
notice-edit-failed-body = { $error }
notice-thumbs-failed-title = Thumbnails couldn't be made
notice-thumbs-failed-body = { $counted ->
    [yes] { $new } new, { $earlier } earlier
   *[no] Some files' thumbnails couldn't be made
}
notice-borders-failed-title = Borders couldn't be looked for
notice-borders-failed-body = { $counted ->
    [yes] { $new } new, { $earlier } earlier
   *[no] Some files couldn't be looked at for their borders
}
notice-import-failed-title = Files couldn't be imported
notice-import-failed-body = { $counted ->
    [yes] { $new } new, { $earlier } earlier
   *[no] Some files couldn't be read to be imported
}
notice-unreadable-files-title = { $pass ->
    [thumbs] Files couldn't be read for their thumbnails
    [borders] Files couldn't be read for their borders
   *[import] Files couldn't be read to be imported
}
notice-unreadable-files-body = { $count ->
    [one] { $text } file couldn't be read (an offline drive, or a file in use): it is tried again by the next pass
   *[other] { $text } files couldn't be read (an offline drive, or files in use): they are tried again by the next pass
}
notice-ledger-not-written-title = { $pass ->
    [thumbs] The thumbnails' failures couldn't be kept
    [borders] The borders' failures couldn't be kept
   *[import] The import's failures couldn't be kept
}
notice-ledger-not-written-body = The files that failed are tried again by the next pass: { $error }
notice-notices-lost-title = Notices weren't kept
notice-notices-lost-body = { $count ->
    [one] { $text } notice couldn't be kept in the inbox; it is in the log
   *[other] { $text } notices couldn't be kept in the inbox; they are in the log
}
notice-file-no-longer-in-library-title = The file is no longer in the library
notice-file-no-longer-in-library-body = It was moved, deleted, or its folder was removed.
notice-update-available-title = A new version is out
notice-update-available-body = Version { $version } is available.
notice-unknown-title = A notice from a newer version
notice-unknown-body = This version of the app can't read it ({ $code }).
# A notice on one line (the command line, a copy of its details)
notice-line = { $title }: { $body }

# The inbox's categories (`pg_model::notice::Source`)
source-scan = Scan
source-import = Import
source-borders = Borders
source-thumbs = Thumbnails
source-crop = Cropping
source-writeback = Writing into files
source-prefs = Preferences
source-catalog = Catalog
source-cache = Thumbnail cache
source-folders = Folders
source-library = Library
source-edit = Edits
source-update = Updates
source-app = App