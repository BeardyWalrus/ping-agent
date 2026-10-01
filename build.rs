fn main() {
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
