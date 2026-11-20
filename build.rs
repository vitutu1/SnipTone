fn main() {
    println!("cargo:rerun-if-changed=assets/sniptone.ico");
    println!("cargo:rerun-if-changed=assets/sniptone_tray_32.bgra");

    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/sniptone.ico");
        res.set("ProductName", "SnipTone");
        res.set(
            "FileDescription",
            "SnipTone — Native HDR Screenshot & Tonemapping HUD",
        );
        res.set("CompanyName", "SnipTone");
        if let Err(err) = res.compile() {
            println!("cargo:warning=Failed to embed Windows icon resource: {err}");
        }
    }
}
