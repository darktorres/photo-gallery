//! The state of the `CommandLine` QObject: the bridge's backing struct. It holds nothing; the command line is read where it is parsed.

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct CommandLineRust;
