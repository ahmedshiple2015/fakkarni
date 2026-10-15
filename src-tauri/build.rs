fn main() {
    // Windows embeds this resource in the executable; source icon changes must
    // invalidate tauri-build's cached resource library, not just the frontend.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    tauri_build::build()
}
