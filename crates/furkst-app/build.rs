//! Embeds the application icon into `furkst.exe` so Explorer and the taskbar show it.

fn main() {
    println!("cargo::rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon.ico");
        resource.compile().expect("embedding the Windows icon (needs rc.exe from the Windows SDK)");
    }
}
