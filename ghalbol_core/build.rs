fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "android" {
        // native_log → __android_log_write
        println!("cargo:rustc-link-lib=log");
    }
}
