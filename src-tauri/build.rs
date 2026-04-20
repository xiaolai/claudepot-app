fn main() {
    // tauri-build does not watch icon files, so dev-mode icon changes
    // don't trigger rebuilds on their own. Watch them explicitly.
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-changed=icons/icon.icns");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=tauri.conf.json");

    tauri_build::build()
}
