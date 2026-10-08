fn main() {
    // Bundled support and standalone preview behavior are separate. A regular
    // release may offer the engine through an explicit runtime setting.
    println!("cargo:rustc-check-cfg=cfg(canna_ducttape_preview)");
    println!("cargo:rustc-check-cfg=cfg(canna_rebound_local_preview)");
    println!("cargo:rerun-if-env-changed=CANNA_DUCTTAPE_SUPPORT");
    println!("cargo:rerun-if-env-changed=CANNA_REBOUND_LOCAL_PREVIEW");
    let local_preview = match std::env::var("CANNA_REBOUND_LOCAL_PREVIEW").as_deref() {
        Ok("1") => true,
        Err(std::env::VarError::NotPresent) | Ok("0") => false,
        _ => panic!("CANNA_REBOUND_LOCAL_PREVIEW must be 0 or 1"),
    };
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("ducttape-support.zip");
    if let Some(bundle) = std::env::var_os("CANNA_DUCTTAPE_SUPPORT") {
        let bundle = std::path::PathBuf::from(bundle);
        println!("cargo:rerun-if-changed={}", bundle.display());
        let bytes = std::fs::read(&bundle).expect("Canna Rebound support bundle");
        assert!(
            bytes.starts_with(b"PK\x03\x04"),
            "Expected ZIP support bundle"
        );
        assert!(
            bytes.len() <= 128 * 1024 * 1024,
            "Support bundle is oversized"
        );
        std::fs::write(&output, bytes).expect("Embed Canna Rebound support bundle");
        println!("cargo:rustc-cfg=canna_ducttape_preview");
        if local_preview {
            println!("cargo:rustc-cfg=canna_rebound_local_preview");
        }
    } else {
        assert!(!local_preview, "A standalone preview needs Rebound support");
        std::fs::write(&output, []).expect("Empty compatibility preview bundle");
    }
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=src/assets/canna.ico");
        println!("cargo:rerun-if-env-changed=CANNA_MAINTENANCE_BUILD");
        println!("cargo:rerun-if-changed=src/bin/canna-updater.rs");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("src/assets/canna.ico");
        if std::env::var_os("CANNA_MAINTENANCE_BUILD").is_some() {
            let source = std::fs::read_to_string("src/bin/canna-updater.rs").unwrap();
            let version = source
                .lines()
                .find(|line| line.starts_with("pub const MAINTENANCE_VERSION"))
                .unwrap()
                .split('"')
                .nth(1)
                .unwrap();
            let parts: Vec<u64> = version
                .split('.')
                .map(|part| part.parse().unwrap())
                .collect();
            let packed = (parts[0] << 48) | (parts[1] << 32) | (parts[2] << 16);
            resource
                .set("ProductName", "Canna Maintenance")
                .set("ProductVersion", version)
                .set("FileVersion", version)
                .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed)
                .set_version_info(winresource::VersionInfo::FILEVERSION, packed);
        }
        resource.compile().expect("Canna executable resource");
    }
}
