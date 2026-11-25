fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../assets/psms.ico");
        winresource::WindowsResource::new()
            .set_icon("../assets/psms.ico")
            .compile()
            .expect("embed the pSMS Windows icon");
    }
}
