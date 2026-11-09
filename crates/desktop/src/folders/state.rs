//! The state of the `FolderModel` QObject: the bridge's backing struct (the folder tree's model state).

#[derive(Default)]
pub(super) struct FolderRust {
    pub(super) ready: bool,
    /// The latest tree asked for; older ones arriving are dropped.
    pub(super) generation: u64,
    /// The tree has been shown once: its first arrival opens the folders above the listed one (`tree_loaded`).
    pub(super) shown: bool,
}
