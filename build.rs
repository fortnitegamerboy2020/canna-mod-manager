fn main() {
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
