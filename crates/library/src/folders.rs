//! A folder added to the library, the same way for the window and the command line: its real path, the scanner's
//! check, and the catalog's row.

use std::path::Path;

use pg_catalog::{Catalog, Driven};
use pg_model::refusal::RefusedReason;
use pg_model::rules::Path as RulePath;
use pg_platform::names::NormalizeError;

/// Why a folder wasn't added. The front end words it: a refusal by its notice (`FolderRefused`), a path by the
/// client's `Message::PathUnreadable` and `Message::NotAPath`; the `Display` form is for the log.
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

/// A removal over the folder that adding it finished first: the removal's folder as the catalog names it, and what the
/// finish did.
#[derive(Debug)]
pub struct Finished {
    pub folder: String,
    pub driven: Driven,
}

/// A folder added over a removal: the folder as the catalog has it, whether it was new, and the removal finished before
/// it (when one was pending).
#[derive(Debug)]
pub struct Added {
    pub root: RulePath,
    pub new: bool,
    pub finished: Option<Finished>,
}

/// Why a folder was not added over a removal. The removal finished before the add is kept in each variant that follows it,
/// so what the finish did is still reported.
#[derive(Debug)]
pub enum AddFinishError {
    /// The add itself failed, as [`add_folder`] says.
    Add { error: AddError, finished: Option<Box<Finished>> },
    /// The finish failed.
    Finish(crate::Error),
    /// The removal is still pending after its finish: files are kept for an unfinished write. Nothing was added.
    StillPending(Finished),
}

/// [`add_folder`], finishing a removal over the folder first: an add over a folder whose removal is not finished is
/// refused, so when there is one, `finish` finishes it (the caller opens the thumbnails the finish needs, which only a
/// pending removal does), and the add is tried again. A removal that is still pending after its finish is not added over.
pub fn add_folder_finishing(
    catalog: &Catalog,
    path: &Path,
    finish: &mut dyn FnMut() -> crate::Result<Driven>,
) -> Result<Added, AddFinishError> {
    let folder = match add_folder(catalog, path) {
        Ok((root, new)) => return Ok(Added { root, new, finished: None }),
        Err(AddError::Catalog(pg_catalog::Error::RemovalPending { path: folder })) => folder,
        Err(error) => return Err(AddFinishError::Add { error, finished: None }),
    };
    let driven = finish().map_err(AddFinishError::Finish)?;
    let finished = Finished { folder, driven };
    match add_folder(catalog, path) {
        Ok((root, new)) => Ok(Added { root, new, finished: Some(finished) }),
        Err(AddError::Catalog(pg_catalog::Error::RemovalPending { .. })) => Err(AddFinishError::StillPending(finished)),
        Err(error) => Err(AddFinishError::Add { error, finished: Some(Box::new(finished)) }),
    }
}
