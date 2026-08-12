use std::env;
use std::path::PathBuf;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        
        let mut icon_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        icon_path.push("assets");
        icon_path.push("plg.ico");
        res.set_icon(icon_path.to_str().unwrap());
        
        res.set_toolkit_path("/usr/bin"); 
        res.set_windres_path("x86_64-w64-mingw32-windres");

        if let Err(e) = res.compile() {
            eprintln!("Failed to compile Windows resources: {:?}", e);
            std::process::exit(1);
        }
    }

    println!("cargo:rerun-if-changed=build.rs");
}
