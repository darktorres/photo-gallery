//! The preferences (`gallery.json`): every choice the app remembers across runs, the front ends' included, saved atomically
//! beside the catalog folder (so they outlive a catalog rebuild). The window's Real Size, the borders shown and the row
//! height are here with the library's write-back and the Yandex folder. The last file selected is here too: it is a
//! file's id in the catalog, so after a rebuild it may name another file (or none).

use serde::{Deserialize, Serialize};

/// The preferences file's content. Fields missing from a file take their defaults; fields the file has that this version
/// doesn't know are ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Write ratings and turns into the files too (on by default); off keeps them in the catalog only.
    pub write_back: bool,
    /// The folder Yandex results were last saved into, empty before the first save.
    pub yandex_folder: String,
    /// The viewer opens images at 1:1 instead of fitted (ImageRater's Real Size), across images and runs.
    pub real_size: bool,
    /// The grid and the viewer show what cropping the borders would cut (B).
    pub show_borders: bool,
    /// The grid's row height in logical pixels.
    pub row_height: f64,
    /// The id of the file selected (or open in the viewer) last, -1 for none.
    pub last_file: i64,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            write_back: true,
            yandex_folder: String::new(),
            real_size: false,
            show_borders: false,
            row_height: 220.0,
            last_file: -1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file's text, pinned: the field names, their order and the number formats are the format on disk.
    const GOLDEN: &str = r#"{
  "write_back": true,
  "yandex_folder": "",
  "real_size": false,
  "show_borders": false,
  "row_height": 220.0,
  "last_file": -1
}"#;

    #[test]
    fn the_defaults_are_written_as_the_golden_text() {
        assert_eq!(serde_json::to_string_pretty(&Prefs::default()).unwrap(), GOLDEN);
    }

    #[test]
    fn the_golden_text_reads_back_to_the_defaults() {
        assert_eq!(serde_json::from_str::<Prefs>(GOLDEN).unwrap(), Prefs::default());
    }

    #[test]
    fn a_changed_value_is_written_as_its_own_text() {
        let prefs = Prefs { real_size: true, row_height: 333.5, last_file: 42, ..Prefs::default() };
        let text = serde_json::to_string(&prefs).unwrap();
        assert_eq!(
            text,
            r#"{"write_back":true,"yandex_folder":"","real_size":true,"show_borders":false,"row_height":333.5,"last_file":42}"#
        );
    }
}
