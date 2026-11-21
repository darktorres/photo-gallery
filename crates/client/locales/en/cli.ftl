# The command line's own lines (pg-client::text::cli): each entry is one `cli::Line`, by its id; a test checks that the
# two lists name the same entries. Its notices are worded by text.ftl's entries, as the window's are. Values are passed
# as text; a line's indentation is the command line's, not part of its words.
# Lines start in lowercase, as the command line's status lines do (the window's words start capitalised). The two that
# begin with a comma are fragments, added after the line before them.

# A pass's progress, on stderr: with no total (files counted as found), and with one
cli-progress-count = { $pass }: { $done } files, { $rate } files/s, { $seconds } s
cli-progress = { $pass }: { $done }/{ $total } ({ $percent }%), { $rate } files/s, { $mbps } MB/s, { $failed } failed, { $seconds } s elapsed, ETA { $eta } s
# A line, with its technical account after it
cli-with-detail = { $line } ({ $detail })
# A build without the app window, started with no command: the one line it prints
cli-no-window = gallery was built without its app window (the `desktop` feature): give it a command
# A file a pass kept as failed: skipped by the scan, or failed by the thumbnails
cli-skipped = skipped: { $path }: { $error }
cli-failed = failed: { $path }: { $error }
# Files that changed while they were read, after a pass's failed count (empty when none)
cli-changed-while-read = , { $count } changed while read, left for the next pass

# gallery scan
cli-scan-summary = { $root }: { $files } files in { $folders } folders, { $changed } new or changed, { $missing } now missing, { $unreadable } unreadable folders ({ $seconds } s)
# A mount under a scanned folder that the scan didn't walk into, with why (the folder ledger has it too)
cli-refused-mount = { $mount } isn't scanned: { $reason }
cli-refused-mounts-not-listed = { $count } more mounts under the scanned folders aren't scanned and aren't listed here; the folder ledger has them
cli-thumb-not-forgotten = { $root }: a replaced file's thumbnail wasn't deleted: { $error }
cli-temp-not-deleted = { $root }: can't delete { $temp }: { $error }

# gallery remove
cli-removed = { $root }: removed from the library, { $deleted } files deleted from the catalog
cli-removal-kept = { $root }: { $deferred } changes kept, waiting to be written. Start the app to finish them, or run remove with --discard-pending to give them up
cli-not-in-library = { $root }: not in the library
cli-scan-busy = a scan of { $root } is running ({ $holder })
cli-scan-removed = { $root } was removed from the library while it was scanned

# gallery import
cli-marked-again = { $count } files to analyze again
cli-import-pending = { $pending } files to import
cli-import-done = imported { $files } files ({ $failed } failed{ $changed }), { $seconds } s
cli-hashes-done = hashed { $files } files ({ $stored } stored, { $failed } failed, { $unreadable } unreadable), { $seconds } s
cli-hashes-owed = { $count } files not hashed yet (run `gallery hash`)
cli-placeholders-done = placeholders made for { $made } files ({ $failed } failed), { $seconds } s
cli-placeholders-owed = { $count } files without a placeholder yet (run `gallery thumbs`)
cli-pair-done = paired { $files } files: { $pairs } PDQ pairs, { $seconds } s
cli-pair-timings = load { $load } s, store { $store } s, join { $join } s (threads' time: sorts { $sort } s, scans { $scan } s, idle { $idle } s, paused { $paused } s), writes { $write } s, commits { $commit } s
cli-pairs-again = { $count } files owe their pairs again (their pairs are dropped)
cli-pairs-legacy = { $count } analyses have no orientation hashes (made by an older build): `gallery import --again` analyzes them again
cli-pairs-owed = { $count } files not paired yet (run `gallery pair`)
# A path named to import again that holds no file of the library
cli-no-file-there = { $path }: no file of the library there

# gallery thumbs
cli-thumbs-needed = { $needed } files needed a { $height } px thumbnail; the cache held { $held } GB
cli-thumbs-done = { $made } thumbnails at { $height } px ({ $failed } failed{ $changed }){ $full }; cache { $used } of { $cap } GB, { $seconds } s
# After the thumbnails' failed count, when the cache filled up
cli-cache-full = , stopped: cache full

# gallery borders
cli-review-imported = { $marked } files marked as not a border; { $unknown } not in the catalog
cli-borders-done = borders looked for in { $files } files ({ $failed } failed, { $stored } stored), { $seconds } s
cli-borders-served = borders looked for in { $count } images
cli-borders-report = { $files } files: { $borders } with borders, { $rejected } rejected candidates only, { $skipped } skipped, { $errors } errors; { $seconds } s ({ $rate } files/s)
cli-csv-missing = --csv: where the report goes

# gallery status
cli-status-home = data folder: { $folder }
cli-status-library = library: { $folder }
cli-status-removing = { $folder }: removal not finished, { $deferred } changes kept, waiting to be written
cli-status-files = { $files } files ({ $missing } missing), { $analyzed } analyzed, { $failed } failed, { $pending } pending, { $pairs } PDQ pairs
cli-status-writes = { $queued } changes to write into files ({ $failing } failing)
cli-status-cache = thumbnail cache: { $count } thumbnails, { $used } of { $cap } GB
cli-status-no-cache = thumbnail cache: none yet

# gallery write
cli-thumbnails-error = thumbnails: file { $file }: { $error }
cli-write-done = written; { $left } left ({ $failing } failing), { $pending } files to re-import (turned), { $seconds } s
cli-writer-stopped = writing into the files has stopped with { $left } changes still to write
cli-writer-stopped-failing = writing into the files has stopped with { $left } changes still to write (the last failure: { $error })

# gallery dups: a tier of identical files or pixels, the PDQ pairs by distance, the groups, and a group's heading
cli-dups-identical = tier { $tier ->
        [file] 0a identical files
       *[pixels] 0b identical pixels
    }: { $groups } groups, { $files } files
cli-dups-pdq = tier 1 PDQ pairs by distance:
cli-dups-band = { $lo }–{ $hi }: { $pairs }
cli-dups-groups = groups (tiers 0a + 0b + PDQ ≤ { $distance }): { $groups } groups, { $files } files, largest { $largest }
cli-dups-group = { $count } files:
cli-file-not-in-catalog = file { $id } isn't in the catalog