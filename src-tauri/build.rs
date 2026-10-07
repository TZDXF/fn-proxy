fn main() {
    // Regenerate native resources when icons change, including the Windows ICO.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
