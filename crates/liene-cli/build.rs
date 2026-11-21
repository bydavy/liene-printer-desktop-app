fn main() {
    // Use the *target* OS, not the host, so cross-compiling to Windows or
    // Linux does not try to pass a macOS `-sectcreate` link flag.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    // Embed an Info.plist so macOS TCC can attribute Bluetooth access to the
    // helper when it is spawned by the GUI app.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let plist = format!("{manifest}/HelperInfo.plist");
    println!("cargo:rerun-if-changed=HelperInfo.plist");
    println!("cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{plist}");
}
