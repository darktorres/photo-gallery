# The words the front end shows (pg-client::text). Each entry is one `text::Message`, by its id; a test checks that the
# two lists name the same entries. Values are passed as text, except `$count`, which chooses the plural form. Spacing
# that must survive (a leading run of spaces) is written as a quoted literal, because Fluent trims a pattern's ends.
# Every id starts with `msg-`, as the `ui-` and `cli-` files' ids do with theirs.

# The facet labels
msg-type-image = Image
msg-type-animated = Animated
msg-type-video = Video
msg-shape-landscape = Landscape
msg-shape-portrait = Portrait
msg-shape-square = Square
msg-status-imported = Imported
msg-status-pending = Not imported yet
msg-status-failed = Unreadable
msg-border-none = None
msg-border-black = Black
msg-border-white = White
msg-border-color = Colored
msg-border-blur = Blurred bars
msg-ext-none = (none)

# The filter line (`text::describe`)
msg-filter-all = all files
msg-filter-unrated = unrated
msg-filter-with-duplicates = with duplicates
msg-filter-without-duplicates = without duplicates
msg-filter-taken = taken { $days }
msg-filter-modified = modified { $days }
# A span of days in the filter line, each end a day (YYYY-MM-DD) or empty
msg-filter-days = { $from } – { $to }
# Between the values of one facet in the filter line, and between the line's parts
msg-filter-list-separator = {", "}
msg-filter-part-separator = {" · "}
msg-filter-range-mb = { $from }–{ $to } MB
msg-filter-range-mp = { $from }–{ $to } MP
msg-filter-range-seconds = { $from }–{ $to } s
msg-filter-borders = borders: { $values }
msg-filter-borders-marked = borders marked not a border
msg-filter-borders-not-marked = borders not marked
msg-filter-name = name has "{ $name }"

# The grid cell's badge. The video's mark is ► (U+25BA), not ▶ (U+25B6): Segoe UI has ► in its bold face, which the badge
# uses, and a glyph the window's font lacks makes Qt load every installed font once (Theme.qml, symbolFamily).
msg-badge-video = ► { $time }
msg-badge-apng = APNG

# The info line
msg-info-not-imported = not imported
msg-info-dims = { $width } × { $height }
msg-info-size-mb = { $size } MB
msg-info-size-kb = { $size } KB
msg-info-kept = { "   " }rating kept in the catalog
msg-info-line = { $path }   { $dims }   { $size }{ $kept }{ $borders }

# The borders of a file (`text::borders_info`)
msg-class-black = black
msg-class-white = white
msg-class-color = colored
msg-class-blur = blurred
msg-borders-info = { "   " }borders { $names } (L { $left } T { $top } R { $right } B { $bottom } px){ $marked }
msg-borders-marked = { " — marked not a border" }

# The status line of a pass (`text::pass_status`)
msg-pass-scan = Scanning { $root }
# A scan's status line: its name (`msg-pass-scan`) and the images found so far
msg-pass-found = { $what } · { $count } images found
msg-pass-import = Importing
msg-pass-hashes = Hashing for duplicates
msg-pass-pairing = Pairing
msg-pass-borders = Finding borders
msg-pass-crop = Saving cropped copies
msg-pass-thumbs = Thumbnails at { $height } px
msg-pass-placeholders = Making ThumbHash placeholders
msg-pass-removing = Removing a folder from the library
msg-pass-working = { $what } { $done } of { $total } · { $rate }/s · { $eta }
# The status bar's link to the pass's failed files, beside its line
msg-pass-failed = { $count } failed

# A time left
msg-eta-seconds = { $seconds } s left
msg-eta-minutes = { $minutes } min left
msg-eta-hours = { $hours } h { $minutes } min left

# The write-back line
msg-writing-changes = { $count ->
    [one] Writing { $text } change into files
   *[other] Writing { $text } changes into files
}
msg-changes-unwritten = { $count ->
    [one] { $text } change not yet in the files: { $error }
   *[other] { $text } changes not yet in the files: { $error }
}
# The latest failure when nothing is left queued (the change it was about went)
msg-write-queue-failed = A change couldn't be written into its file: { $error }

# Why the scanner refuses a folder (`pg_model::refusal::RefusedReason`), selected by its code: the `$reason` of the
# notice `folder-refused`. `$file_system` is a file system's name, `$error` the system's error.
msg-refused-reason = { $code ->
    [network-path] it is a network (UNC) path
    [unreadable-volume] its volume can't be read: { $error }
    [unreadable-folder] the folder can't be read: { $error }
    [unreadable-real-path] the folder's real path can't be read: { $error }
    [unreadable-real-volume] its real volume can't be read: { $error }
    [unreadable-file-system] its file system can't be read: { $error }
    [unreadable-drive] its disk can't be read: { $error }
    [network-drive] its drive is a network drive
    [removable-drive] its drive is removable
    [other-drive] its drive is not a fixed local disk
    [link] it is a link (a junction or a symbolic link)
    [real-not-local] its real location is not on a local drive
    [real-on-other-volume] its real location is on another volume
    [not-ntfs] its file system is { $file_system }, not NTFS
    [opaque-disk] its disk is virtual, part of Storage Spaces or a RAID, an iSCSI disk, or on a bus that can't be told: only plain physical disks are supported
    [nfs] it is on an NFS mount
    [smb] it is on an SMB or CIFS share
    [fat] its file system is FAT, whose file numbers change when it is mounted again
    [ex-fat] its file system is exFAT, whose file numbers change when it is mounted again
    [pseudo] it is the kernel's { $file_system } file system, not a disk
    [unsupported-system] scanning is supported on Windows and Linux only
   *[other] it can't be scanned for its files' identities
}
# A file a job panicked on (caught: the job went on with the next file), as a crop's or a write's failure
msg-file-panicked = internal error on this file: { $message }
# The error a job reported on a file, in the system's or the library's words, as the detail under the failure.
msg-failure-detail = Error: { $error }

# The Yandex panel
msg-yandex-captcha = Yandex asked for a captcha, which the app can't answer. Try again later.
msg-yandex-not-still = Yandex search takes still images (JPEG, PNG, WebP, GIF, BMP or TIFF).
msg-yandex-uploading = Uploading the photo…
msg-yandex-other-sizes = Reading the other sizes…
msg-yandex-similar = Reading similar images…
msg-yandex-unreadable-photo = Can't read the photo: { $error }
msg-yandex-similar-stopped = Similar images stopped after { $shown } results: { $reason }
msg-yandex-similar-failed = Similar images failed: { $reason }
msg-yandex-not-started = Couldn't start the search or the save: { $error }
msg-folder-dialog-failed = Couldn't open the folder dialog: { $error }
msg-folder-unreadable = Couldn't read the folder: { $error }
# A path given to add (a folder, or a file named on the command line) that can't be opened, or isn't one
msg-path-unreadable = Can't open { $path }: { $error }
msg-not-a-path = { $path } is not a path
msg-yandex-nothing-picked = Pick at least one image to save.
msg-yandex-no-folder = No folder was chosen, so nothing was saved.

# A lane's row while the lane is paused by itself: no pass running, a pass with its count, or a scan with what it found
msg-lane-paused = Paused
msg-paused-pass = Paused · { $status }
msg-lane-paused-pass = Paused · { $what } { $done } of { $total }
msg-lane-paused-found = Paused · { $what } · { $count } images found

# The Yandex panel's search and save lines
msg-yandex-starting = Starting…
msg-yandex-saving = { $count ->
    [one] Saving { $text } file…
   *[other] Saving { $text } files…
}
msg-yandex-saving-file = Saving { $n } of { $of }…
msg-yandex-saved = { $count ->
    [one] Saved { $text } file to { $folder }
   *[other] Saved { $text } files to { $folder }
}
msg-yandex-saved-failed = Saved { $written }, { $failed } failed. First failure: { $error }
msg-yandex-download-failed = { $url } couldn't be downloaded: { $error }
msg-yandex-write-failed = { $url } couldn't be written to { $path }: { $error }
# A search or a save stopped by a panic, which the app logged
msg-yandex-panicked = Ended by an internal error; the details are in { $log }.
msg-folder-list-changed = The folder list changed since it was shown: nothing was removed, and the list is read again. Try again.
# Why a Yandex request failed (`pg_yandex::YandexError`; the captcha is `msg-yandex-captcha`)
msg-yandex-http-status = Yandex answered HTTP { $status }
msg-yandex-transport = Yandex request failed: { $error }
msg-yandex-upload-not-json = Yandex's answer to the upload is not JSON
msg-yandex-upload-no-cbir-id = Yandex's answer to the upload has no image id
msg-yandex-upload-no-original = Yandex's answer to the upload has no original picture
msg-yandex-results-page-no-state = Yandex's results page has no search state
msg-yandex-results-state-not-json = Yandex's search state is not JSON
msg-yandex-similar-not-json = Yandex's similar-images answer is not JSON
msg-yandex-similar-no-results = Yandex's similar-images answer has no result list
msg-yandex-not-an-image = Yandex's result is not an image
msg-yandex-too-large = Yandex's picture is larger than the download limit
msg-yandex-no-result = Yandex's search has no such result
msg-yandex-no-search = There is no search to take a result from
msg-yandex-no-search-on-screen = Nothing is on screen to save
msg-yandex-not-called = The request was never made
msg-yandex-recorded = recorded Yandex session: { $error }
msg-yandex-undecodable = a result's file is not an image this app can decode: { $reason }

# The boxes the app shows before it ends: the catalog that can't be opened, and a panic (`text::internal_error`: the
# first line, the panic's message, then where its details are)
msg-catalog-unopened = Can't open the catalog: { $error }
msg-catalog-newer-schema = The catalog at { $path } was made by a later build (its schema is version { $found }; this build's is { $current }){ $built_by }. Open it with that build, or restore a backup by hand (rename it over catalog.sqlite).
msg-catalog-newer-mark = The catalog at { $path } was marked by a later build ({ $mark } is { $found }; this build knows up to { $current }){ $built_by }. Open it with that build, or restore a backup by hand (rename it over catalog.sqlite).
msg-catalog-older-schema = The catalog at { $path } has schema version { $found }, and this build needs version { $current }. A released catalog is not migrated: restore a backup by hand (rename it over catalog.sqlite).
msg-catalog-drift = The catalog at { $path } has a schema (version { $found }) that differs from this build's: { $detail }. A released catalog is not rebuilt: restore a backup by hand (rename it over catalog.sqlite).
msg-catalog-predates-key-version = The catalog at { $path } has a schema that differs from this build's, and it predates key_version. Restore a .pre-v*.bak or start fresh.
msg-catalog-path-rules = The catalog at { $path } was { $detail }, but this build uses the { $wanted } path rules. Open it on the platform that made it, or move it aside and scan again.
msg-catalog-in-use = Another program has the catalog at { $path } open, so it can't be changed until it is closed.
msg-catalog-folder-missing = The catalog folder { $path } was not found. Is its drive connected? Choose a folder that exists, or use the default.
msg-catalog-read-only =The catalog at { $path } needs a change, but it can't be written (it, or its folder, is read-only or permission is denied). Make it writable, or open a copy of it.
msg-catalog-path-too-long =The catalog's folder path is too long for Windows: { $path }. Choose a shorter folder for the catalog (its files are kept beside it).
msg-internal-error = Photo Gallery hit an internal error:
msg-details-in-log = The details are in { $log }.
msg-details-not-logged = Writing them to { $log } failed ({ $error }).

# The notices (`pg_model::notice::Notice`): each kind's title (the inbox row, the toast) and body (the details), by its
# code. `$count` chooses a plural form and `$text` shows it; a path is as the system writes it, isolated from the
# words around it (U+2068 … U+2069, set with the value: `text/notice.rs`). The kinds retired (`scan-unreadable`,
# `mount-refused`, `root-refused`: now the folders of `scan-folders-failed`) have no words.
msg-notice-scan-failed-title = A scan failed
msg-notice-scan-failed-body = { $root }: { $error }
msg-notice-scan-note-title = { $kind ->
    [duplicates] Duplicate paths skipped
    [reparse] Links indexed without being followed
    [ambiguous] Files that couldn't be matched
    [unpaired] Missing files kept
    [replaced] Files replaced
    [rebound] Files found again
    [volume-id] Volumes without a UUID
   *[other] A scan note
}
msg-notice-scan-note-body = { $root }: { $kind ->
    [duplicates] { $text } files were skipped as duplicates (another path to the same file)
    [reparse] { $text } links or placeholders were indexed without being followed
    [ambiguous] { $text } files matched more than one earlier row, so nothing was carried over
    [unpaired] { $text } missing files kept their ratings; no file was found for them
    [replaced] { $text } files were replaced by new files with the same name; their content was reset
    [rebound] { $text } files were found under a new identity and kept their ratings
    [volume-id] { $text } volumes have no file system UUID, so their files are told apart by a device number that a restart can renumber
   *[other] { $text } scan notes
}
msg-notice-round-failed-title = The background job failed
msg-notice-round-failed-body = { $error }
msg-notice-round-panicked-title = Internal error in the background job
msg-notice-round-panicked-body = { $message }. The job went on with the next request; the details are in the log.
msg-notice-cache-full-title = The thumbnail cache is full
msg-notice-cache-full-body = { $made } thumbnails at { $height } px weren't made. Make the cache bigger, or let it drop old thumbnails.
msg-notice-cropped-title = Cropped copies saved
msg-notice-cropped-body = { $saved } saved in { $folder }; { $skipped } had nothing to crop; { $failed ->
    [0] none failed
   *[other] { $failed } failed, the first: { $path }: { $error }
}
msg-notice-crop-failed-title = Saving cropped copies failed
msg-notice-crop-failed-body = { $error }
msg-notice-borders-cut-title = Borders cut from the originals
msg-notice-borders-cut-body = { $saved } cut in place; { $skipped } had nothing to cut; { $unsupported ->
    [0] none skipped for their format;
   *[other] { $unsupported } skipped, a format that is not cut in place;
} { $failed ->
    [0] none failed
   *[other] { $failed } failed, the first: { $path }: { $error }
}
msg-notice-cut-failed-title = Cutting the borders failed
msg-notice-cut-failed-body = { $error }
msg-notice-catalog-rebuilt-title = The catalog was rebuilt
msg-notice-catalog-rebuilt-body = It didn't match this version of the app ({ $why }), so a new one was made and the old one kept as { $kept }. { $carried ->
    [rows] It took the old one's { $files } files and { $folders } library folders.
   *[nothing] It took nothing from the old one ({ $reason }): the next scan fills it.
}
msg-notice-catalog-not-closed-title = The catalog wasn't closed
msg-notice-catalog-not-closed-body = { $error }
msg-notice-prefs-unreadable-title = The preferences couldn't be read
msg-notice-prefs-unreadable-body = { $path } didn't parse ({ $error }), so the defaults are in use. { $kept ->
    [none] It couldn't be kept aside: { $keep_error }
   *[other] It was kept as { $kept }.
}
msg-notice-prefs-not-saved-title = The preferences couldn't be saved
msg-notice-prefs-not-saved-body = { $path }: { $error }. The change holds until the app closes.
msg-notice-cache-totals-fixed-title = The thumbnail cache's totals were fixed
msg-notice-cache-totals-fixed-body = They said { $kept_count } thumbnails in { $kept_bytes } bytes; the cache holds { $count } in { $bytes } bytes.
msg-notice-cache-touches-not-written-title = The thumbnail cache couldn't note what was shown
msg-notice-cache-touches-not-written-body = { $error }. It tries again later.
msg-notice-cache-not-stored-title = A thumbnail couldn't be kept
msg-notice-cache-not-stored-body = { $error }. It was shown all the same.
msg-notice-cache-not-swept-title = The thumbnails of removed files couldn't be dropped
msg-notice-cache-not-swept-body = { $error }. They are dropped at the next try.
msg-notice-cache-not-read-title = The thumbnail cache couldn't be read
msg-notice-cache-not-read-body = { $error }. Its thumbnails wait until it can be read.
msg-notice-cache-wal-not-merged-title = The thumbnail cache wasn't closed cleanly
msg-notice-cache-wal-not-merged-body = { $error }. It is merged when the app starts again.
msg-notice-cache-started-over-title = The thumbnail cache was started over
msg-notice-cache-started-over-body = { $reason ->
    [damaged] The cache was damaged, so it was started over; its thumbnails are made again. The old file is kept as { $path }
    [not-a-database] The cache was not a database, so it was started over; its thumbnails are made again. The old file is kept as { $path }
    [other-catalog] The cache { $path } was made for another catalog, so it was started over; its thumbnails are made again
   *[other-build] The cache { $path } was made by another version of the app, so it was started over; its thumbnails are made again
}
msg-notice-cache-disabled-title = Thumbnails aren't kept
msg-notice-cache-disabled-body = The thumbnail cache couldn't be opened, so each thumbnail is made when it is shown: { $error }
msg-notice-write-failed-title = A change couldn't be written into its file
msg-notice-write-failed-body = { $file ->
    [none] { $error }. It is tried again later.
   *[other] { $file } may be incomplete: copying its change into it failed ({ $error }). Its whole new content is in { $temp }, and the copy is tried again.
}
msg-notice-write-stopped-title = Writing into the files has stopped
msg-notice-write-stopped-body = Writing changes into the files has stopped after repeated failures. The changes are kept, and are written when the app starts again.
msg-notice-write-dropped-title = Queued changes were dropped
msg-notice-write-dropped-body = { $count ->
    [one] { $kind ->
        [rating] A queued rating was dropped: its file is no longer in the library
       *[rotation] A queued turn was dropped: its file is no longer in the library
    }
   *[other] { $kind ->
        [rating] { $text } queued changes were dropped, the latest a rating: their files are no longer in the library
       *[rotation] { $text } queued changes were dropped, the latest a turn: their files are no longer in the library
    }
}
msg-notice-folder-already-added-title = { $folder } is already in the library
msg-notice-folder-already-added-body = It is scanned again.
msg-notice-folder-refused-title = The folder can't be added
msg-notice-folder-refused-body = { $folder } can't be added: { $reason }
msg-notice-folder-not-added-title = The folder couldn't be added
msg-notice-folder-not-added-body = { $error }
msg-notice-folder-not-removed-title = The folder couldn't be removed from the library
msg-notice-folder-not-removed-body = { $error }
msg-notice-folder-removal-unfinished-title = A change to a file in { $folder } is waiting to be written
msg-notice-folder-removal-unfinished-body = The file was deleted before its change was written, and its folder is being removed from the library. The new content is kept in { $temp }; the change is not written anywhere else.
msg-notice-rescan-failed-title = The library couldn't be scanned again
msg-notice-rescan-failed-body = { $error }
msg-notice-library-unavailable-title = The background work couldn't start
msg-notice-library-unavailable-body = Nothing is scanned, imported or written into the files: { $error }
msg-notice-changes-unseen-title = The window can't see the library's changes
msg-notice-changes-unseen-body = Changes made in the background won't show until the app is started again: { $error }
msg-notice-folders-unreadable-title = The library's folders couldn't be read
msg-notice-folders-unreadable-body = { $error }
msg-notice-picker-failed-title = The folder dialog failed
msg-notice-picker-failed-body = { $error }
msg-notice-listing-failed-title = The files couldn't be listed
msg-notice-listing-failed-body = { $error }
msg-notice-row-read-failed-title = A file's details couldn't be read
msg-notice-row-read-failed-body = { $error }
msg-notice-reload-failed-title = A changed file couldn't be read again
msg-notice-reload-failed-body = { $error }
msg-notice-facets-failed-title = The filters' counts couldn't be read
msg-notice-facets-failed-body = { $error }
msg-notice-tree-failed-title = The folder tree couldn't be read
msg-notice-tree-failed-body = { $error }
msg-notice-views-failed-title = { $op ->
    [list] The saved views couldn't be listed
    [save] The view couldn't be saved
   *[delete] The view couldn't be deleted
}
msg-notice-views-failed-body = { $error }
msg-notice-session-not-saved-title = Where you were couldn't be saved
msg-notice-session-not-saved-body = { $error }
msg-notice-session-not-read-title = Where you were couldn't be read
msg-notice-session-not-read-body = { $error }
msg-notice-edit-failed-title = { $op ->
    [rate] A rating couldn't be saved
    [rotate] A turn couldn't be saved
   *[mark-border] A border mark couldn't be saved
}
msg-notice-edit-failed-body = { $error }
msg-notice-thumbs-failed-title = Thumbnails couldn't be made
msg-notice-thumbs-failed-body = { $counted ->
    [yes] { $new } new, { $earlier } earlier
   *[no] Some files' thumbnails couldn't be made
}
msg-notice-borders-failed-title = Borders couldn't be looked for
msg-notice-borders-failed-body = { $counted ->
    [yes] { $new } new, { $earlier } earlier
   *[no] Some files couldn't be looked at for their borders
}
msg-notice-import-failed-title = Files couldn't be imported
msg-notice-import-failed-body = { $counted ->
    [yes] { $new } new, { $earlier } earlier
   *[no] Some files couldn't be read to be imported
}
msg-notice-scan-folders-failed-title = Folders couldn't be scanned
msg-notice-scan-folders-failed-body = { $counted ->
    [yes] { $new } new, { $earlier } earlier
   *[no] Some folders couldn't be read, or the scan refuses them, so their files aren't in the library
}
msg-notice-unreadable-files-title = { $pass ->
    [thumbs] Files couldn't be read for their thumbnails
    [borders] Files couldn't be read for their borders
   *[import] Files couldn't be read to be imported
}
msg-notice-unreadable-files-body = { $count ->
    [one] { $text } file{ $where } couldn't be read (an offline drive, or a file in use): it is tried again by the next pass
   *[other] { $text } files{ $where } couldn't be read (an offline drive, or files in use): they are tried again by the next pass
}
msg-notice-ledger-not-written-title = The thumbnails' failures couldn't be kept
msg-notice-ledger-not-written-body = The files that failed are tried again by the next pass: { $error }
msg-notice-notices-lost-title = Notices weren't kept
msg-notice-notices-lost-body = { $count ->
    [one] { $text } notice couldn't be kept in the inbox; it is in the log
   *[other] { $text } notices couldn't be kept in the inbox; they are in the log
}
msg-notice-file-no-longer-in-library-title = The file is no longer in the library
msg-notice-file-no-longer-in-library-body = It was moved, deleted, or its folder was removed.
msg-notice-update-available-title = A new version is out
msg-notice-update-available-body = Version { $version } is available.
msg-notice-unknown-title = A notice from a newer version
msg-notice-unknown-body = This version of the app can't read it ({ $code }).
# A notice on one line (the command line, a copy of its details)
msg-notice-line = { $title }: { $body }
# A ledger's entry that came back after the user dismissed it: how many of its failures came since (`$count`)
msg-ledger-since-dismissed = { $count ->
    [one] { $text } since you dismissed it
   *[other] { $text } since you dismissed it
}
# A ledger's failed file, one line of what the inbox's Copy puts on the clipboard
msg-failed-file-line = { $path }: { $error }
# Why a folder of the scans' ledger failed, by its reason's code (`pg_catalog::FolderFailure::code`)
msg-failed-folder-reason = { $reason ->
    [unreadable] it couldn't be read
    [mount-refused] it is a mount the scan refuses
   *[refused] the scan refuses it
}
# A folder of the scans' ledger, one line of the Copy: under its library folder's line (`msg-failed-folder-group`)
msg-failed-folder-line = { $path }: { $why } ({ $error })
msg-failed-folder-group = In { $root }:

# The inbox's categories (`pg_model::notice::Source`)
msg-source-scan = Scan
msg-source-import = Import
msg-source-borders = Borders
msg-source-thumbs = Thumbnails
msg-source-crop = Cropping
msg-source-writeback = Writing into files
msg-source-prefs = Preferences
msg-source-catalog = Catalog
msg-source-cache = Thumbnail cache
msg-source-folders = Folders
msg-source-library = Library
msg-source-edit = Edits
msg-source-update = Updates
msg-source-app = App
