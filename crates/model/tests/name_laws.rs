//! Laws of `pg_model::name::Name` (S-17.1), over arbitrary UTF-16 units and arbitrary bytes: the exact round trip,
//! canonical construction (equal names have equal bytes and the reverse), injective name keys, and decoding that
//! never panics. Nothing here touches the file system: names are built from units and bytes.

use pg_model::name::Name;
use proptest::prelude::*;

/// UTF-16 with surrogates of both kinds likely: the units a Windows name can hold.
fn wide() -> impl Strategy<Value = Vec<u16>> {
    prop::collection::vec(
        prop_oneof![
            4 => any::<u16>(),
            2 => 0xD800u16..=0xDBFF,
            2 => 0xDC00u16..=0xDFFF,
            1 => 0x20u16..0x7F,
        ],
        0..40,
    )
}

proptest! {
    #[test]
    fn a_wide_name_gives_its_units_back(units in wide()) {
        let name = Name::from_wide(&units);
        // Canonical bytes read back as the same name.
        prop_assert_eq!(Name::from_wtf8(name.as_bytes()), Some(name.clone()));
        // The units round trip, except that a valid pair is one scalar value (it comes back as the same pair).
        prop_assert_eq!(name.to_wide(), units);
    }

    #[test]
    fn equal_names_have_equal_bytes_and_the_reverse(a in wide(), b in wide()) {
        let (na, nb) = (Name::from_wide(&a), Name::from_wide(&b));
        prop_assert_eq!(na == nb, na.as_bytes() == nb.as_bytes());
        // Two names are equal exactly when their UTF-16 forms are (the wide form is injective on built names).
        prop_assert_eq!(na == nb, na.to_wide() == nb.to_wide());
    }

    #[test]
    fn name_keys_are_injective(a in wide(), b in wide()) {
        let (na, nb) = (Name::from_wide(&a), Name::from_wide(&b));
        prop_assert_eq!(na.name_key() == nb.name_key(), na == nb);
    }

    #[test]
    fn name_keys_order_case_insensitively_then_exactly(a in "[a-zA-Z0-9Σσς .]{0,12}", b in "[a-zA-Z0-9Σσς .]{0,12}") {
        let (na, nb) = (Name::from(a.as_str()), Name::from(b.as_str()));
        let folded = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<String>();
        let expected = (folded(&a), a.as_bytes()).cmp(&(folded(&b), b.as_bytes()));
        // The key's order is the spec's order: lower case first, then the exact bytes. (A string's key is the same
        // with the leading 0x00, so the comparison is on the body.)
        let body = |n: &Name| {
            let key = n.name_key();
            let end = key[1..].iter().position(|&x| x == 0).unwrap() + 1;
            (key[1..end].to_vec(), key[end + 1..].to_vec())
        };
        let (ka, kb) = (body(&na), body(&nb));
        prop_assert_eq!(ka.cmp(&kb), expected);
    }

    #[test]
    fn any_bytes_decode_and_round_trip_as_raw(bytes in prop::collection::vec(1u8..=255, 0..40)) {
        let name = Name::from_raw(&bytes);
        prop_assert_eq!(name.as_bytes(), bytes.as_slice());
        // Every byte is in exactly one unit, and the units do not panic on any input.
        let _ = name.display();
        let _ = name.name_key();
        let _ = name.units().count();
    }

    #[test]
    fn serde_round_trips_any_name(units in wide()) {
        let name = Name::from_wide(&units);
        let json = serde_json::to_string(&name).unwrap();
        prop_assert_eq!(serde_json::from_str::<Name>(&json).unwrap(), name);
    }
}
