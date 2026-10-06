use std::{env, fs, path::PathBuf};

fn main() {
    embed(
        "canna-update-token.txt",
        "canna_update_token.rs",
        "EMBEDDED_UPDATE_TOKEN",
    );
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
