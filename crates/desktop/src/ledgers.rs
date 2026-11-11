//! A failure ledger as QML names it: a number (a `Notices` row's `pass`, a bit of `Notices.passes`,
//! `Library.failedPass`, `FailedFiles.show`, `--inbox-expand`). A leaf, which the inbox (`notices`), the files list it
//! expands (`failed_files`) and the library's status (`library`) share.

use pg_model::notice::LedgerKind;

/// A ledger's code in QML: its place in [`LedgerKind::ALL`], -1 for none. A pass's ledger has the pass's place in
/// `FilePass::ALL`, which `LedgerKind::ALL` lists first.
pub(crate) fn kind_code(kind: Option<LedgerKind>) -> i32 {
    kind.and_then(|k| LedgerKind::ALL.iter().position(|&q| q == k)).and_then(|i| i32::try_from(i).ok()).unwrap_or(-1)
}

/// The ledger of a QML code ([`kind_code`]).
pub(crate) fn kind_of(code: i32) -> Option<LedgerKind> {
    usize::try_from(code).ok().and_then(|i| LedgerKind::ALL.get(i)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::notice::FilePass;

    /// The codes QML holds: a pass's ledger keeps the pass's place, the scans' folders come after, and none is -1.
    #[test]
    fn a_ledgers_code_is_its_place() {
        assert_eq!(kind_code(Some(LedgerKind::File(FilePass::Thumbs))), 0);
        assert_eq!(kind_code(Some(LedgerKind::File(FilePass::Import))), 2);
        assert_eq!(kind_code(Some(LedgerKind::ScanFolders)), 3);
        assert_eq!(kind_code(None), -1);
        for kind in LedgerKind::ALL {
            assert_eq!(kind_of(kind_code(Some(kind))), Some(kind));
        }
        assert_eq!(kind_of(-1), None);
        assert_eq!(kind_of(99), None);
        for (i, pass) in FilePass::ALL.into_iter().enumerate() {
            assert_eq!(kind_code(Some(LedgerKind::File(pass))), i32::try_from(i).unwrap());
        }
    }
}
