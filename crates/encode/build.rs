fn main() {
    println!("cargo:rustc-link-lib=avcodec");
    println!("cargo:rustc-link-lib=avutil");
    println!("cargo:rustc-link-lib=swscale");

    cc::Build::new()
        .file("c_hw.c")
        .compile("gs_hw");
}
