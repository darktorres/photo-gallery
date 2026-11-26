//! The upcase table the Windows name key maps each UTF-16 unit through (`PathRules::name_key`), as NTFS's `$UpCase`
//! does: one unit to one unit, BMP only, the simple uppercase mapping where it round-trips (the uppercase lowercases
//! back to the unit), from Unicode 5.1, which is the table Windows writes on an NTFS volume (checked over the whole BMP
//! on one, `tools/gen_upcase.py`). So `ς`, `ı`, `ſ` and `µ` are their own uppercase, as on NTFS. The table is embedded
//! and versioned (`upcase_table.rs`, made by the generator from the Unicode data), so a key is the same on every machine
//! and toolchain; its hash is part of the catalog's `key_version` mark.

#[path = "upcase_table.rs"]
mod table;

/// The Unicode release the table was made from.
pub const UCD_RELEASE: &str = table::UCD_RELEASE;

/// The table's identity: FNV-1a over its pairs. A changed table changes every key it maps.
pub const TABLE_HASH: u64 = table::TABLE_HASH;

/// The uppercase of one UTF-16 unit: its pair in the table, or itself (a unit with no single-unit uppercase in the BMP,
/// a surrogate, a supplementary letter's halves).
pub fn upcase(unit: u16) -> u16 {
    if unit < 0x80 {
        // ASCII: the table's pairs there are exactly a-z to A-Z.
        return u16::from(u8::try_from(unit).unwrap_or(0).to_ascii_uppercase());
    }
    match table::PAIRS.binary_search_by_key(&unit, |&(from, _)| from) {
        Ok(i) => table::PAIRS.get(i).map_or(unit, |&(_, to)| to),
        Err(_) => unit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_maps_each_unit_once() {
        assert!(table::PAIRS.windows(2).all(|w| matches!(w, [a, b] if a.0 < b.0)));
        assert!(table::PAIRS.iter().all(|&(from, to)| from != to && !(0xD800..=0xDFFF).contains(&to)));
    }

    #[test]
    fn ascii_and_the_table_agree() {
        for unit in 0u16..0x80 {
            let expected = table::PAIRS.iter().find(|p| p.0 == unit).map_or(unit, |p| p.1);
            assert_eq!(upcase(unit), expected, "{unit:#x}");
        }
    }

    /// The answers an NTFS volume gave (gate G2 of `docs/plans/CATALOG_SIZE_PLAN.md`, Windows 10 19045, every BMP
    /// character): what it folds, and what it keeps apart.
    #[test]
    fn the_cases_ntfs_folds_by_one_unit() {
        // σ to Σ, é to É, ÿ to Ÿ, the dz digraph to its capital, Latin Extended-C and Cyrillic Extended-B (Unicode 5.1).
        for (unit, upper) in
            [(0x03C3, 0x03A3), (0x00E9, 0x00C9), (0x00FF, 0x0178), (0x01C6, 0x01C4), (0x0250, 0x2C6F), (0xA641, 0xA640)]
        {
            assert_eq!(upcase(unit), upper, "{unit:#x}");
        }
        // Kept apart: a mapping that doesn't round-trip (the final sigma, dotless i, long s, micro sign, the titlecase
        // dz, the beta symbol: their uppercase lowercases to another letter), a case pair given after Unicode 5.1
        // (Latin Extended-C's 5.2 letters, Georgian Mtavruli, Cherokee small letters, Latin Extended-D), no single-unit
        // uppercase (sharp s, the dotted capital I, the Kelvin sign), and the surrogates.
        for unit in [
            0x03C2, 0x0131, 0x017F, 0x00B5, 0x01C5, 0x03D0, 0x0252, 0x10D0, 0x13F8, 0xA791, 0x00DF, 0x0130, 0x212A,
            0xD801, 0xDC28,
        ] {
            assert_eq!(upcase(unit), unit, "{unit:#x}");
        }
    }
}
