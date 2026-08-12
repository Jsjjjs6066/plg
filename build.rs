use std::{
    env,
};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/plg.ico");
        res.compile().unwrap();
    }

    println!("cargo:rerun-if-changed=build.rs");
}
