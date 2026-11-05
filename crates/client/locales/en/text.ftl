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
pass-scan = Scanning { $root } · { $count } images found
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

# The job's notices
unreadable-folders = { $root }: { $count } folders couldn't be read
scan-failed = { $root }: { $error }
root-refused = { $root } is not scanned, because { $reason }. Remove it from the library to stop this notice.
scan-note = { $root }: { $kind ->
    [duplicates] { $count } files were skipped as duplicates (another path to the same file)
    [reparse] { $count } links or placeholders were indexed without being followed
    [ambiguous] { $count } files matched more than one earlier row, so nothing was carried over
    [unpaired] { $count } missing files kept their ratings; no file was found for them
    [replaced] { $count } files were replaced by new files with the same name; their content was reset
    [rebound] { $count } files were found under a new identity and kept their ratings
   *[other] { $count } scan notes
}
round-panicked = internal error in the background job: { $message }
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
yandex-not-still = Yandex search takes still images (JPEG, PNG, WebP, GIF or BMP).
yandex-uploading = Uploading the photo…
yandex-other-sizes = Reading the other sizes…
yandex-similar = Reading similar images…
yandex-unreadable-photo = Can't read the photo: { $error }
yandex-similar-stopped = Similar images stopped after { $shown } results: { $reason }
yandex-similar-failed = Similar images failed: { $reason }
