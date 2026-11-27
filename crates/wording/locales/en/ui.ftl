# The words of the front end's own literals: the labels, tooltips, prompts and titles that QML shows. Each entry
# is read by `text::ui` through `Words.text(id)`, or through `Words.phrase(id, count, text, other)` when it has a value:
# `$count` chooses a plural form and is not shown; `$text` and `$other` are shown as they are, so a number is passed
# as text (Fluent may group its digits). The ids start with `ui-`; a test checks that every id the QML uses is here.
# Spacing that must survive is written in the QML around the word, not in the entry.

# The filter bar
ui-any = Any
ui-with-duplicates = With duplicates
ui-without-duplicates = Without duplicates
ui-not-marked = Not marked
ui-marked-not-a-border = Marked not a border
ui-min = min
ui-max = max
ui-date-format = YYYY-MM-DD
ui-type = Type
ui-rating = Rating
ui-unrated = Unrated
ui-extension = Extension
ui-shape = Shape
ui-size-mb = Size (MB)
ui-megapixels = Megapixels
ui-duration-s = Duration (s)
ui-taken = Taken
ui-modified = Modified
ui-duplicates = Duplicates
ui-import = Import
ui-borders = Borders
ui-review = Review
ui-name = Name
ui-save-crops = Save cropped copies ({ $text })…
ui-crop-tooltip = Copies of the listed files with their borders cut off, in a folder you choose (the originals stay as they are). JPEGs are cropped losslessly; their left and top cuts move inward to the next 8 or 16 px block.
ui-cut-originals = Cut borders from originals…
ui-cut-tooltip = Rewrites the listed originals with their borders cut off, in place. No backup copy is kept. JPEGs are re-encoded at their own quality; TIFFs are re-encoded losslessly with their metadata kept; lossy WebPs stay lossy. Files with nothing to cut, or a format that isn't cut, are skipped.
ui-cut-borders = Cut borders
ui-cut-borders-title = Cut borders from the originals?
ui-cut-borders-question = Cut the borders off { $text } { $count ->
    [one] file?
   *[other] files?
} { $count ->
    [one] Its original will be rewritten in place.
   *[other] Their originals will be rewritten in place.
} No backup copy is kept.
ui-contains = contains…
ui-clear = Clear

# The grid's empty state
ui-no-folders =
    No folders in the library yet.
    Add one with Library… (top right).
ui-nothing-matches = Nothing here matches the filters.
ui-nothing-importing = Nothing here yet: the library is being imported.
ui-no-images = No images here.
ui-listing-failed = The images here couldn't be read.
ui-grid-position = file ≈ { $text } of { $other }

# The viewer's info line
ui-real-size = [real size]
ui-loupe = [loupe 10×]
ui-turning = — turning…
ui-original-unavailable = — original unavailable
ui-player-error = — { $text }
ui-player-error-playback = Playback failed

# The library panel
ui-library-folders = Library folders
ui-library-intro = Images in these folders and their subfolders are imported in the background. Removing a folder deletes its images from the catalog, ratings and thumbnails included; the files on disk are not touched.
ui-remove = Remove
ui-remove-folder-title = Remove this folder from the library?
ui-remove-folder-question = Its images are deleted from the catalog, with their ratings, analyses and thumbnails. The files on disk are not touched, and the folder can be added again later.
ui-removal-heading = Being removed
ui-removal-removing = removing
ui-removal-kept = kept: a change is waiting to be written
ui-discard-removal = Discard
ui-discard-removal-title = Discard this removal?
ui-discard-removal-question = The changes not yet written into its files are given up, and their temporary copies are deleted. The files they were being written into may be left partly written.
ui-discard-removal-confirm = Discard changes
ui-no-folders-yet = No folders yet.
ui-write-files = Write ratings and turns into the files too (where the format allows)
ui-write-files-tooltip = Ratings are always kept in the catalog. With this on, they also go into the image files (XMP), so other programs see them; turns rotate the files losslessly.
ui-detect-borders = Detect borders in the background
ui-detect-borders-tooltip = Finds the borders of each image, for cropping them off and the border filters. Off: no new borders are found; the ones found stay.
ui-detect-duplicates = Find duplicates in the background
ui-detect-duplicates-tooltip = Hashes each image and pairs it with the others, for the duplicates views. Off: new images are not hashed or paired, and the hashes left out are made when it is back on.
ui-thumb-cache = Keep thumbnails in the cache
ui-thumb-cache-tooltip = Off: every thumbnail is decoded from its original each time it is shown, which is slower, and no new placeholders are made. What the cache holds is kept, and it is used again when this is on.
ui-borders-disabled = Border detection is off. Turn it on in the Library panel.
ui-duplicates-disabled = Duplicate detection is off. Turn it on in the Library panel.
ui-clear-filters-tooltip = Clear all filters
ui-options-borders-disabled = A view or a start option uses borders, and border detection is off, so it was left out. Turn it on in the Library panel.
ui-nothing-to-crop = No file in the list has borders to crop.
ui-add-folder = Add folder…
ui-rescan-all = Rescan all
ui-close = Close
# The status bar's button that opens the inbox, with how many of its entries are unread (none: no count)
ui-inbox = { $count ->
    [0] Inbox
   *[other] Inbox ({ $text })
}
ui-inbox-title = Inbox
ui-inbox-empty = Nothing to tell
ui-inbox-clear = Clear all
ui-inbox-copy = Copy
ui-inbox-dismiss = Dismiss
ui-inbox-retry = Retry
# Shows a pass's failed files: the inbox's button, and the status bar's link to them (the pass line's words are pass-failed)
ui-inbox-show-files = Show the failed files
ui-inbox-reveal = Reveal
# A ledger's entry lists its newest failed files: how many it holds ($text) and how many are listed ($other)
ui-inbox-more = The newest { $other } of { $text } files
# The scans' ledger lists its folders: how many it holds ($text) and how many are listed ($other)
ui-inbox-more-folders = { $other } of { $text } folders
# A ledger's list when it couldn't be read ($text: why); while it is read, the shared ui-loading
ui-inbox-unreadable = The list couldn't be read: { $text }
# The pass's "N failed" when the inbox lists nothing of it yet (its failures aren't recorded yet): plain text, this its tooltip
ui-failed-not-recorded = Not recorded in the inbox yet
# The log view (dev mode): its button in the status bar, which is also its title, and what it shows with no record
ui-log = Log
ui-log-empty = Nothing logged yet
# The letters that mark a record's level in the log view: error, warning and info
ui-log-level-error = E
ui-log-level-warning = W
ui-log-level-info = I

# The window and its dialogs
ui-app-name = Photo Gallery
ui-window-title = Photo Gallery — { $text }
ui-save-view-title = Save the current view
ui-all-folders = All folders
# The folder tree's row of the files directly in a drive root (D:\, /), which no folder holds.
ui-drive-roots = Drive roots
ui-and-subfolders = and subfolders
ui-view-name = Name (one already saved is replaced)

# The rating bar
ui-find-similar-tooltip = Find similar images on Yandex
ui-unmark-not-border = Marked not a border: unmark (N)
ui-not-a-border = Not a border (N)
ui-turn-left = Turn left (R)
ui-turn-right = Turn right (Shift+R)
ui-stars = { $count ->
    [0] Unrated (0)
    [one] { $text } star ({ $text })
   *[other] { $text } stars ({ $text })
}

# The status bar
ui-pause = Pause
ui-resume = Resume
# A lane's own pause, named for its drive (`$text`): the accessible name of its row's button
ui-pause-lane = Pause { $text }
ui-resume-lane = Resume { $text }

# The top bar
ui-sort-name = By name
ui-sort-file-type = By file type
ui-sort-file-size = By file size
ui-sort-modified = By date modified
ui-sort-taken = By date taken
ui-sort-created = By date created
ui-sort-rating = By rating
ui-sort-megapixels = By megapixels
ui-sort-width = By width
ui-sort-height = By height
ui-sort-aspect-ratio = By aspect ratio
ui-sort-duration = By duration
ui-sort-frames = By frames
ui-sort-border-depth = By border depth
ui-sort-descending = Descending (click for ascending)
ui-sort-ascending = Ascending (click for descending)
ui-loading = Loading…
ui-files = { $text } files
ui-files-of = { $text } of { $other } files
ui-include-subfolders = Include subfolders
ui-filters = Filters
ui-filters-count = Filters ({ $text })
ui-filters-tooltip = Filter by type, extension, rating, size, date, duplicates… (F)
ui-library = Library…
ui-size = Size
ui-px = px

# The video transport
ui-play-pause = Play / pause (K)
ui-mute = Mute (M)

# The views menu
ui-views = Views ▾
ui-save-current-view = Save current view…
ui-delete = Delete
ui-delete-view = Delete view
ui-delete-view-title = Delete this saved view?
ui-delete-view-question = The view is removed from the saved views. Nothing in the library changes.

# The Yandex panel and preview
ui-select-none = Select none
ui-select-all = Select all

# The app's own folder dialog (where the system's dialog is not used)
ui-choose-folder-title = Choose a folder
ui-folder-up = Up
ui-choose = Choose
ui-choose-here = Choose this folder
ui-cancel = Cancel
ui-save = Save
ui-find-similar-yandex = Find similar on Yandex
ui-save-picked = Save { $text } { $count ->
    [one] image…
   *[other] images…
}
ui-other-sizes = Other sizes of this photo
ui-similar-images = Similar images
ui-no-results = No results.
ui-yandex-hint = Double-click a result to see it at full size.
ui-other-size = Other size { $text } of { $other }
ui-similar-image = Similar image { $text } of { $other }
ui-loading-percent = Loading… { $text }%
ui-cannot-decode = Can't load or decode it; saving may still work.
ui-picked = Picked ✓
ui-pick = Pick
ui-fit = Fit
ui-fit-undecodable = — this file cannot be decoded
ui-fit-rewriting = — the file is being written; try again shortly
ui-region-failed = — this part couldn't be shown
ui-one-to-one = 1:1
ui-back = Back
# The key hints: each label of the grid's and the viewer's hint line (`keys::Hint`), joined by the separator
ui-hint-separator = { " · " }
ui-hint-select = Click to select
ui-hint-view = double-click, Enter or Space to view
ui-hint-rate = 0–5 rate
ui-hint-turn = R / Shift+R turn
ui-hint-arrows = arrows move
ui-hint-filters = F filters
ui-hint-borders = B borders
ui-hint-not-border = N not a border
ui-hint-resize = Ctrl+wheel or Ctrl+/− resize
ui-hint-step = ← → step
ui-hint-play = K play/pause
ui-hint-mute = M mute
ui-hint-up-four = ↑ 4
ui-hint-down-one = ↓ 1
ui-hint-turn-back = R ⟲
ui-hint-turn-on = Shift+R ⟳
ui-hint-zoom = pinch or + − zoom
ui-hint-one-to-one = double-click 1:1
ui-hint-real-size = Z real size
ui-hint-loupe = L loupe
ui-hint-hide = H hide
ui-hint-close = Esc close
ui-details-unavailable = Details unavailable
ui-count-pending = …
ui-counting = counting…
ui-dev-reload-failed = The QML did not load. Fix the file and save it, and the window loads again, or quit.
ui-quit = Quit
# The selection's status line: the item's info, then its rating symbol
ui-selection-line = { $text }{ "   " }{ $other }
