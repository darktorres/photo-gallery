//! The state of the `Settings` QObject: the bridge's backing struct. It holds nothing; the preferences are the app's.

/// The properties' state is the preferences': the struct holds nothing.
#[derive(Default)]
pub struct SettingsRust;
