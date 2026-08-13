use std::{env};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        embed_resource::compile("icon.rc", embed_resource::NONE);
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/plg.ico");
}
