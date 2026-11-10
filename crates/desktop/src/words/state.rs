//! The state of the `Words` QObject: the bridge's backing struct. It holds nothing; the words are the client's.

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct WordsRust;
