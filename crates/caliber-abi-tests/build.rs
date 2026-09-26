use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let caliber_root = manifest_dir.join("../..");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    cc::Build::new()
        .file(manifest_dir.join("tests/abi/layout_probe.c"))
        .file(manifest_dir.join("tests/abi/old_client.c"))
        .file(manifest_dir.join("tests/abi/current_client.c"))
        .include(caliber_root.join("include"))
        .include(manifest_dir.join("tests/fixtures/v1-prefix"))
        .warnings(true)
        .std("c11")
        .compile("caliber_abi_clients");

    if target_os == "linux" {
        println!("cargo:rustc-link-lib=dl");
    }

    println!(
        "cargo:rerun-if-changed={}",
        caliber_root.join("include/caliber.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("tests/abi").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir
            .join("tests/fixtures/v1-prefix/caliber_v1_prefix.h")
            .display()
    );
}
