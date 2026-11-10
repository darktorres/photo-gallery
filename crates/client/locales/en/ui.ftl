# The words of the front end's own literals: the labels, tooltips, prompts and titles that QML shows (S-19.2). Each entry
# is read by `text::ui` through `Words.text(id)`, or through `Words.phrase(id, count, text, other)` when it has a value:
# `$count` chooses a plural form and is not shown; `$text` and `$other` are shown as they are, so a number is passed
# as text (Fluent may group its digits). The ids start with `ui-`; a test checks that every id the QML uses is here.
# Spacing that must survive is written in the QML around the word, not in the entry.

# The filter bar
ui-any = Any
ui-has-duplicates = Has duplicates
ui-no-duplicates = No duplicates
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
ui-contains = contains…
ui-clear = Clear

# The grid's empty state
ui-no-folders =
    No folders in the library yet.
    Add one with Library… (top right).
ui-nothing-matches = Nothing here matches the filters.
ui-nothing-importing = Nothing here yet: the library is being imported.
ui-no-images = No images here.
ui-grid-position = file ≈ { $text } of { $other }

# The viewer's info line
ui-real-size = [real size]
ui-loupe = [loupe 10×]
ui-turning = — turning…
ui-original-unavailable = — original unavailable
ui-player-error = — { $text }

# The library panel
ui-library-folders = Library folders
ui-library-intro = Images in these folders and their subfolders are imported in the background. Removing a folder only hides its images: adding it back restores them, ratings included.
ui-remove = Remove
ui-no-folders-yet = No folders yet.
ui-write-files = Write ratings and turns into the files too (where the format allows)
ui-write-files-tooltip = Ratings are always kept in the catalog. With this on, they also go into the image files (XMP), so other programs see them; turns rotate the files losslessly.
ui-add-folder = Add folder…
ui-rescan-all = Rescan all
ui-close = Close
# The log view (dev mode): its button in the status bar, its title, and what it shows with no record
ui-log = Log
ui-log-title = Log
ui-log-empty = Nothing logged yet

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
ui-delete-view = Delete view

# The Yandex panel and preview
ui-select-none = Select none
ui-select-all = Select all

# The app's own folder dialog (where the system's dialog is not used)
ui-choose-folder-title = Choose a folder
ui-folder-up = Up
ui-choose = Choose
ui-choose-here = Choose this folder
ui-cancel = Cancel
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
ui-back = Back
