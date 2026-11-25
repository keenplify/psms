fn main() {
    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
            return;
        }
        println!("cargo:rerun-if-changed=assets/psms.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/psms.ico")
            .compile()
            .expect("embed the pSMS Windows icon");
    }
}
