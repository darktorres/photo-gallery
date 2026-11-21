//! Environment flags: the one reader of an on/off variable, so each flag takes the same spellings everywhere.

/// The on/off value of the environment variable `name`: `Some(true)` for `1`, `true`, `yes` or `on`, `Some(false)` for
/// `0`, `false`, `no` or `off` (see [`parse`]). `None` when the variable is unset, not valid UTF-8, or set to anything
/// else; the caller decides whether to say so.
pub fn flag(name: &str) -> Option<bool> {
    parse(&std::env::var(name).ok()?)
}

/// The on/off value of a flag's text: `1`, `true`, `yes` or `on` is `Some(true)`, `0`, `false`, `no` or `off` is
/// `Some(false)`, and anything else is `None`. The match ignores case and surrounding spaces.
pub fn parse(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each on and off spelling reads as its value, in any case and with spaces around it; anything else reads as
    // `None`, so a misspelt flag is not taken as either.
    #[test]
    fn a_flag_reads_its_on_and_off_spellings() {
        for on in ["1", "true", "TRUE", " Yes ", "on", "On"] {
            assert_eq!(parse(on), Some(true), "{on:?}");
        }
        for off in ["0", "false", "No", "OFF", "off "] {
            assert_eq!(parse(off), Some(false), "{off:?}");
        }
        for other in ["", "maybe", "2", "enabled", "y"] {
            assert_eq!(parse(other), None, "{other:?}");
        }
    }
}
