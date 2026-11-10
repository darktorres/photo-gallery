//! The state of the `Stages` QObject: the bridge's backing struct. It holds nothing; the stages are `pg-trace`'s.

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct StagesRust;
