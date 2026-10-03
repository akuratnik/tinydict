// macOS only grants microphone access to a launchd job whose binary declares
// why it wants it. A bare CLI has no bundle, so embed the Info.plist directly.
fn main() {
    println!("cargo:rerun-if-changed=macos/Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let plist = concat!(env!("CARGO_MANIFEST_DIR"), "/macos/Info.plist");
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{plist}");
    }
}
