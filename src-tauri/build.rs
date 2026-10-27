fn main() {
    println!("cargo:rerun-if-changed=src/media/native.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/media/native.m")
            .flag("-fobjc-arc")
            .flag("-Wno-deprecated-declarations")
            .compile("gphotos_media");
        for framework in [
            "Foundation",
            "ImageIO",
            "CoreGraphics",
            "AVFoundation",
            "CoreMedia",
            "CoreVideo",
            "Vision",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
    tauri_build::build()
}
