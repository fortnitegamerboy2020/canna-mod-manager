use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Default)]
pub struct Editor {
    selected: Option<PathBuf>,
    text: String,
    hash: String,
    status: String,
}
pub fn scope(game: &crate::model::InstalledGame) -> PathBuf {
    if game.app_id == u32::MAX {
        game.path.join("config")
    } else if let Some(addons) = crate::model::source_addons(game.app_id) {
        game.path
            .join(Path::new(addons).parent().unwrap())
            .join("cfg")
    } else {
        game.path.join("BepInEx/config")
    }
}
fn permitted(root: &Path, path: &Path) -> Result<()> {
    crate::runtime::no_links(root)?;
    crate::runtime::no_links(path)?;
    anyhow::ensure!(
        path.canonicalize()?.starts_with(root.canonicalize()?)
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("cfg")
                    || e.eq_ignore_ascii_case("toml")
                    || e.eq_ignore_ascii_case("json")),
        "Choose a config inside this game's mod config folder"
    );
    anyhow::ensure!(
        fs::metadata(path)?.is_file() && fs::metadata(path)?.len() <= 1024 * 1024,
        "Config exceeds limits"
    );
    Ok(())
}
pub fn write(root: &Path, path: &Path, expected: &str, text: &str) -> Result<()> {
    permitted(root, path)?;
    anyhow::ensure!(text.len() <= 1024 * 1024, "Config exceeds limits");
    let old = fs::read(path)?;
    anyhow::ensure!(
        format!("{:x}", Sha256::digest(&old)) == expected,
        "Config changed outside Canna; reload before saving"
    );
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        let _: serde_json::Value =
            serde_json::from_str(text).context("Invalid JSON configuration")?;
    }
    let backup = path.with_extension("canna-config-backup");
    let pending = path.with_extension("canna-config-pending");
    crate::runtime::no_links(&backup)?;
    crate::runtime::no_links(&pending)?;
    fs::write(&backup, old)?;
    fs::write(&pending, text)?;
    fs::rename(pending, path)?;
    Ok(())
}
impl Editor {
    pub fn show(
        &mut self,
        ui: &mut eframe::egui::Ui,
        game: &crate::model::InstalledGame,
        busy: bool,
    ) {
        let root = scope(game);
        ui.label("Local configuration editor. Changes are backed up beside the file; no values are shared.");
        if ui.button("Choose configuration…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_directory(&root)
                .add_filter("Configurations", &["cfg", "toml", "json"])
                .pick_file()
        {
            match permitted(&root, &path).and_then(|_| {
                let bytes = fs::read(&path)?;
                self.hash = format!("{:x}", Sha256::digest(&bytes));
                self.text = String::from_utf8(bytes).context("Configuration must be UTF-8 text")?;
                self.selected = Some(path);
                Ok(())
            }) {
                Ok(()) => self.status = "Config loaded locally".into(),
                Err(e) => self.status = e.to_string(),
            }
        }
        if let Some(path) = &self.selected {
            ui.label(path.file_name().unwrap_or_default().to_string_lossy());
            ui.add(
                eframe::egui::TextEdit::multiline(&mut self.text)
                    .code_editor()
                    .desired_rows(12)
                    .char_limit(1024 * 1024),
            );
            if ui
                .add_enabled(!busy, eframe::egui::Button::new("Save configuration"))
                .clicked()
            {
                self.status = match crate::runtime::ensure_closed(game)
                    .and_then(|_| write(&root, path, &self.hash, &self.text))
                {
                    Ok(()) => {
                        self.hash = format!("{:x}", Sha256::digest(self.text.as_bytes()));
                        "Configuration saved; previous contents are backed up".into()
                    }
                    Err(e) => e.to_string(),
                };
            }
        }
        ui.label(&self.status);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_external_changes_and_invalid_json() {
        let root = std::env::temp_dir().join(format!("canna-config-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("config.json");
        fs::write(&file, b"{}").unwrap();
        let hash = format!("{:x}", Sha256::digest(b"{}"));
        assert!(write(&root, &file, &hash, "invalid").is_err());
        assert_eq!(fs::read(&file).unwrap(), b"{}");
        write(&root, &file, &hash, "{\"value\":true}").unwrap();
        assert_eq!(
            fs::read(file.with_extension("canna-config-backup")).unwrap(),
            b"{}"
        );
        assert!(write(&root, &file, &hash, "{}").is_err());
        assert!(write(&root.join("other"), &file, &hash, "{}").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
