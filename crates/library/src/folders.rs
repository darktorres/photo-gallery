//! A folder added to the library, the same way for the window and the command line: its real path, the scanner's
//! check, and the catalog's row.

use std::path::Path;

use pg_catalog::Catalog;
use pg_model::refusal::RefusedReason;
use pg_model::rules::Path as RulePath;
use pg_platform::names::NormalizeError;

/// Why a folder wasn't added. The front end words it: a refusal by its notice (`FolderRefused`), a path by the
/// client's `path_unreadable` and `not_a_path`; the `Display` form is for the log.
#[derive(Debug, thiserror::Error)]
pub enum AddError {
    /// The scanner refuses the folder (`pg_scan::check_folder`): it would stay in the library with a notice on every
    /// scan.
    #[error("{} can't be added: {reason}", pg_platform::native::PATH_RULES.display(folder))]
    Refused { folder: RulePath, reason: RefusedReason },
    /// The path can't be opened, or isn't one.
    #[error(transparent)]
    Path(#[from] NormalizeError),
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
}

/// Add the folder at `path` (exact) to the library: its real path (`names::normalize_path`), checked by the scanner's
/// rule, then recorded. The folder as the catalog has it, and whether it was new; one already in the library is no
/// error. Nothing is scanned: the caller does that.
pub fn add_folder(catalog: &Catalog, path: &Path) -> Result<(RulePath, bool), AddError> {
    let root = pg_platform::names::normalize_path(path)?;
    if let Err(reason) = pg_scan::check_folder(catalog.rules(), &root) {
        return Err(AddError::Refused { folder: root, reason });
    }
    let added = catalog.add_root(&root)?;
    Ok((root, added))
}
