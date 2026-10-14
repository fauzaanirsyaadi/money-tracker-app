// NDK r27 links with 4 KB segment alignment by default, but Android 15+ devices
// may use 16 KB pages and Google Play requires 16 KB support. Set it explicitly
// so the result does not depend on the NDK version.
#[allow(clippy::print_stdout, reason = "cargo build-script directive")]
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,max-page-size=16384");
    }
    tauri_build::build()
}
