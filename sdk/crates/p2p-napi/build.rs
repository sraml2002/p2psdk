fn main() {
    let sdk_path = std::env::var("OHOS_NDK_HOME")
        .unwrap_or_else(|_| "/Users/sram/Library/OpenHarmony/Sdk/20".to_string());
    println!("cargo:rustc-link-search=native={}/native/sysroot/usr/lib/aarch64-linux-ohos", sdk_path);
    println!("cargo:rustc-link-lib=ace_napi.z");
    println!("cargo:rustc-link-lib=hilog_ndk.z");
}
