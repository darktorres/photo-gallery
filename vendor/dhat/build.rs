// Walks frame pointers only where the walk can read the frame records and
// the thread's stack, and only when the whole program keeps them: on x86_64
// and aarch64, on Linux and macOS, with `-C force-frame-pointers=yes` among
// the build's flags. Any other build, Windows included, unwinds with
// `backtrace`.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(dhat_frame_pointers)");
    println!("cargo::rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let flags: Vec<&str> = flags.split('\x1f').collect();
    let forced = flags.iter().enumerate().any(|(i, flag)| {
        let value = match flag.strip_prefix("-C") {
            Some("") => flags.get(i + 1).copied().unwrap_or(""),
            Some(rest) => rest,
            None => flag.strip_prefix("--codegen=").unwrap_or(""),
        };
        matches!(
            value,
            "force-frame-pointers=yes"
                | "force-frame-pointers=y"
                | "force-frame-pointers=on"
                | "force-frame-pointers=true"
        )
    });
    let target = |name: &str| std::env::var(name).unwrap_or_default();
    let known = matches!(target("CARGO_CFG_TARGET_OS").as_str(), "linux" | "macos")
        && matches!(
            target("CARGO_CFG_TARGET_ARCH").as_str(),
            "x86_64" | "aarch64"
        );
    if forced && known {
        println!("cargo::rustc-cfg=dhat_frame_pointers");
    }
}
