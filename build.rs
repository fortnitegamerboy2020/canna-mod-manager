use std::{env, fs, path::PathBuf};

fn main() {
    embed(
        "canna-update-token.txt",
        "canna_update_token.rs",
        "EMBEDDED_UPDATE_TOKEN",
    );
    println!("cargo:rerun-if-changed=canna-token.txt");
    let path = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("canna-token.txt");
    let token = match fs::read_to_string(path) {
        Ok(value) => value
            .trim()
            .trim_start_matches('\u{feff}')
            .trim()
            .to_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => panic!("Could not read canna-token.txt"),
    };
    assert!(
        !token.chars().any(char::is_whitespace),
        "canna-token.txt must contain only one token"
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("canna_token.rs");
    fs::write(
        output,
        format!("const EMBEDDED_GITHUB_TOKEN: &str = {token:?};\n"),
    )
    .expect("Could not generate embedded token configuration");
}

fn embed(file: &str, output: &str, constant: &str) {
    println!("cargo:rerun-if-changed={file}");
    let path = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join(file);
    let token = fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .trim_start_matches('\u{feff}')
        .trim()
        .to_owned();
    assert!(
        !token.chars().any(char::is_whitespace),
        "Credential file must contain one token"
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(output),
        format!("const {constant}: &str = {token:?};\n"),
    )
    .expect("Could not generate updater credential configuration");
}
