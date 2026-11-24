fn main() {
    // Re-run the build script when icons change; otherwise the exe keeps stale resources.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    tauri_build::build()
}
