//! A walk's lease as its write transactions check it ([`HeldLease`]): `core` hands it out (`ScanLease::held`), and the
//! scan's writer (`writers`) and the pairing of moved files (`pair`) check it. Below all three, so none of them reaches
//! back into another for it.

use rusqlite::Connection;

use pg_model::ids::ScanId;
use pg_model::rules::{Path as RulePath, PathRules};

use crate::error::{Error, Result};

/// A walk's lease, as its write transactions check it: its scan, and the folder it walks, under `rules`.
#[derive(Clone, Copy)]
pub(crate) struct HeldLease<'a> {
    pub(crate) scan: ScanId,
    pub(crate) rules: PathRules,
    pub(crate) root: &'a RulePath,
}

impl HeldLease<'_> {
    /// The lease is still the walk's: its heartbeat is now, written on `conn` (in its transaction, when one is open: the
    /// first statement of each of the walk's write transactions, so what the transaction writes commits only with the
    /// lease held). A lease that is gone writes nothing: [`Error::RootRemoved`] when no library folder holds the walked
    /// folder any more (its removal took the lease), [`Error::LeaseLost`] otherwise (it expired, and another walk may
    /// have begun).
    pub(crate) fn hold(&self, conn: &Connection) -> Result<()> {
        let held = conn
            .prepare_cached("UPDATE scan_leases SET heartbeat = unixepoch() WHERE id = ?1")?
            .execute([self.scan.get()])?;
        if held > 0 {
            return Ok(());
        }
        let root = self.rules.display(self.root);
        Err(match crate::types::covering_root(conn, self.rules, self.root)? {
            None => Error::RootRemoved { root },
            Some(_) => Error::LeaseLost { root },
        })
    }
}
