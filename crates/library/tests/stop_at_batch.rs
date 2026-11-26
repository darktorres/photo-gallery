//! The passes that a toggle or a zoom can end: each one ends after the batch in which its `still_wanted` turns false, and
//! what it did not reach stays owed (or pending, or due) for the next run. Each test has more files than one batch
//! (`N`), so a stop has work left to show.

mod common;

use std::path::PathBuf;

use common::{Result, opts, scanned};
use pg_catalog::Catalog;
use pg_jobs::control::Control;

/// More files than one batch of the import, the hashes and the borders (500 each): a stop after the first batch leaves
/// some. If a batch grows past this, the tests fail, and this number is the one to raise.
const N: usize = 501;

/// A run folder with `N` pictures of 64 × 64 (the border detector looks at those), scanned into a catalog beside them,
/// not imported: the catalog, the run's control, and the folder.
fn pictures(prefix: &str) -> Result<(Catalog, Control, PathBuf)> {
    // A pattern, not a flat colour: the picture is a real one for every pass.
    let pixels: Vec<u8> = (0..64 * 64 * 3)
        .map(|i| u8::try_from(i % 251))
        .collect::<std::result::Result<_, std::num::TryFromIntError>>()?;
    let png = pg_codec::fixtures::png_rgb(64, 64, &pixels)?;
    scanned(prefix, |images| {
        for i in 0..N {
            std::fs::write(images.join(format!("{i:04}.png")), &png)?;
        }
        Ok(())
    })
}

#[test]
fn an_import_stops_after_its_batch_and_leaves_the_rest_pending() -> Result<()> {
    let (mut c, ctl, _dir) = pictures("import")?;
    let stats = pg_import::import_while(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| false, &ctl).unwrap();
    assert!(stats.files > 0 && stats.files < N, "one batch was imported: {}", stats.files);
    let rest = c.pending_import(&pg_catalog::VolumeSet::All).unwrap().items.len();
    assert_eq!(stats.files + rest, N, "the files not reached stay pending");
    pg_import::import_while(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert!(c.pending_import(&pg_catalog::VolumeSet::All).unwrap().items.is_empty(), "the next run imports the rest");
    Ok(())
}

#[test]
fn the_hashes_pass_stops_after_its_batch_and_keeps_the_rest_owed() -> Result<()> {
    let (mut c, ctl, _dir) = pictures("hashes")?;
    pg_import::import_while(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert_eq!(c.owed_hashes().unwrap(), u64::try_from(N).unwrap(), "every picture owes its hashes");
    let stats = pg_import::hashes::hash(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| false, &ctl).unwrap();
    assert!(!stats.complete, "a stopped pass is not complete");
    assert!(stats.stored > 0 && stats.stored < N, "one batch was hashed: {}", stats.stored);
    assert_eq!(c.owed_hashes().unwrap(), u64::try_from(N - stats.stored).unwrap(), "the files not reached stay owed");
    let done = pg_import::hashes::hash(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert!(done.complete);
    assert_eq!(c.owed_hashes().unwrap(), 0, "the next run hashes the rest");
    Ok(())
}

#[test]
fn the_borders_pass_stops_after_its_batch_and_keeps_the_rest_due() -> Result<()> {
    let (mut c, ctl, _dir) = pictures("borders")?;
    pg_import::import_while(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert_eq!(
        c.pending_borders(&pg_catalog::VolumeSet::All).unwrap().items.len(),
        N,
        "every picture is due its borders"
    );
    let stats = pg_borders::find_while(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| false, &ctl).unwrap();
    assert!(!stats.complete, "a stopped pass is not complete");
    let rest = c.pending_borders(&pg_catalog::VolumeSet::All).unwrap().items.len();
    assert!(rest > 0 && rest < N, "one batch was looked at; {rest} are still due");
    pg_borders::find_while(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert!(c.pending_borders(&pg_catalog::VolumeSet::All).unwrap().items.is_empty(), "the next run looks at the rest");
    Ok(())
}
