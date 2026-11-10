fn main() {
    // CI's run number is the build number the self-update compares against
    // (src/update.rs). A local build has none and reports build 0.
    println!("cargo:rerun-if-env-changed=GITHUB_RUN_NUMBER");
    if let Ok(n) = std::env::var("GITHUB_RUN_NUMBER") {
        if n.trim().parse::<u32>().is_ok() {
            println!("cargo:rustc-env=PINGAGENT_BUILD={}", n.trim());
        }
    }
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        if std::path::Path::new("assets/app.ico").exists() {
            res.set_icon("assets/app.ico");
        }
        res.set_manifest_file("assets/app.manifest");
        res.set("ProductName", "PingAgent");
        res.set("FileDescription", "PingAgent - system tray ping monitor");
        if let Err(e) = res.compile() {
            println!("cargo:warning=failed to embed Windows resources: {e}");
        }
    }
}
