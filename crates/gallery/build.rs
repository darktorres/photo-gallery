fn main() {
    // The icon of gallery.exe, which Explorer and the taskbar show (made by tools/make_icon.py). The resource compiler
    // is part of the Windows SDK. The target's OS, not this script's host, decides: the script's cfg is the host's.
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        println!("cargo:rerun-if-changed=resources/app.ico");
        let mut res = winres::WindowsResource::new();
        res.set_icon("resources/app.ico");
        res.compile().expect("embedding resources/app.ico in gallery.exe");
    }
}
