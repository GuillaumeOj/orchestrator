fn main() {
    println!("cargo:rerun-if-env-changed=ORCH_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=ORCH_BUILD_CHANNEL");
    tauri_build::build()
}
