//! The state of the `Settings` QObject: the bridge's backing struct. It holds nothing; the preferences are the app's.

/// The properties' state is the preferences': the struct holds nothing.
#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct SettingsRust;
