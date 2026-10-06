fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=src/assets/canna.ico");
        winresource::WindowsResource::new()
            .set_icon("src/assets/canna.ico")
            .compile()
            .expect("Canna executable icon");
    }
}
