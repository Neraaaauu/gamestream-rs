use std::fs;
use std::path::Path;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let syslib_dir = Path::new(&out_dir).join("syslibs");
    let _ = fs::create_dir_all(&syslib_dir);

    let links = [
        ("/usr/lib/x86_64-linux-gnu/libpulse-simple.so.0", "libpulse-simple.so"),
        ("/usr/lib/x86_64-linux-gnu/libpulse.so.0", "libpulse.so"),
        ("/usr/lib/x86_64-linux-gnu/libopus.so.0", "libopus.so"),
    ];

    for (target, name) in links {
        let dest = syslib_dir.join(name);
        if !dest.exists() {
            let _ = std::os::unix::fs::symlink(target, dest);
        }
    }

    println!("cargo:rustc-link-search=native={}", syslib_dir.display());
    println!("cargo:rustc-link-lib=pulse-simple");
    println!("cargo:rustc-link-lib=pulse");
    println!("cargo:rustc-link-lib=opus");
}
