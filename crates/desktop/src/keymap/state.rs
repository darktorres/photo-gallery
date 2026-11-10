//! The state of the `KeyMap` QObject: the bridge's backing struct. It holds nothing; the keys are the controller's.

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct KeyMapRust;
