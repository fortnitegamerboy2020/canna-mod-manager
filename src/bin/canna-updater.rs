#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[path = "../updater.rs"]
#[allow(dead_code)] // Shared launcher API includes its current-executable convenience wrapper.
mod updater;
use anyhow::{Context, Result};
use eframe::egui;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};
pub const MAINTENANCE_VERSION: &str = "0.1.0";
const APP: &str = "Canna Mod Manager.exe";
const UPDATER: &str = "Canna Updater.exe";
#[cfg(windows)]
fn ps(script: &str) -> Result<String> {
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x08000000)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "Windows could not check the application state"
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
#[cfg(not(windows))]
fn ps(_: &str) -> Result<String> {
    anyhow::bail!("Canna maintenance requires Windows")
}
fn literal(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "''"))
}
fn ensure_closed(target: &Path) -> Result<()> {
    let script = format!(
        "Get-CimInstance Win32_Process | Where-Object {{ $_.ExecutablePath -eq {} }} | ForEach-Object {{ $_.ProcessId }}",
        literal(target)
    );
    anyhow::ensure!(
        ps(&script)?.is_empty(),
        "Close Canna before updating or restoring it, then try again"
    );
    Ok(())
}
fn installed_version(target: &Path) -> String {
    ps(&format!(
        "(Get-Item -LiteralPath {}).VersionInfo.ProductVersion",
        literal(target)
    ))
    .ok()
    .filter(|s| s.split('.').count() == 3 && s.split('.').all(|part| part.parse::<u32>().is_ok()))
    .unwrap_or_else(|| "0.0.0".into())
}
fn verified_rollback(target: &Path, stage: &Path) -> Result<updater::Ready> {
    let backup = target.with_extension("previous.exe");
    let expected = fs::read_to_string(backup.with_extension("exe.sha256"))
        .context("No verified rollback copy is available yet")?;
    let meta = fs::metadata(&backup)?;
    anyhow::ensure!(
        meta.len() > 0 && meta.len() <= 150 * 1024 * 1024,
        "Invalid rollback size"
    );
    let bytes = fs::read(&backup)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    anyhow::ensure!(
        bytes.starts_with(b"MZ") && hash == expected.trim(),
        "Rollback checksum mismatch; use Repair instead"
    );
    fs::create_dir_all(stage.parent().context("Invalid staging path")?)?;
    fs::write(stage, bytes)?;
    Ok(updater::Ready {
        version: "previous".into(),
        file: stage.to_owned(),
        hash,
    })
}
#[derive(Clone, Copy)]
enum Operation {
    AppUpdate,
    AppRepair,
    UpdaterUpdate,
    UpdaterRepair,
    AppRestore,
    UpdaterRestore,
}
struct Maintenance {
    root: PathBuf,
    recovery: bool,
    status: String,
    pending: Option<Receiver<Result<Option<updater::Ready>>>>,
    target: Option<PathBuf>,
}
impl Maintenance {
    fn start(&mut self, operation: Operation) {
        let app = matches!(
            operation,
            Operation::AppUpdate | Operation::AppRepair | Operation::AppRestore
        );
        let target = self.root.join(if app { APP } else { UPDATER });
        if app && let Err(error) = ensure_closed(&target) {
            self.status = error.to_string();
            return;
        }
        if !app
            && self.recovery
            && let Err(error) = ensure_closed(&target)
        {
            self.status = error.to_string();
            return;
        }
        let current = if app || self.recovery {
            installed_version(&target)
        } else {
            MAINTENANCE_VERSION.to_owned()
        };
        self.status = "Checking cannamods.vip and verifying the download…".into();
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.target = Some(target.clone());
        std::thread::spawn(move || {
            let result = match operation {
                Operation::AppUpdate => updater::check(&current),
                Operation::AppRepair => updater::check("0.0.0"),
                Operation::UpdaterUpdate => updater::check_maintenance(&current),
                Operation::UpdaterRepair => updater::check_maintenance("0.0.0"),
                Operation::AppRestore | Operation::UpdaterRestore => {
                    let local = std::env::var_os("LOCALAPPDATA")
                        .map(PathBuf::from)
                        .unwrap_or_else(std::env::temp_dir);
                    verified_rollback(
                        &target,
                        &local.join("CannaModManager/updates").join(format!(
                            "restore-{}-{}.exe",
                            if app { "app" } else { "updater" },
                            std::process::id()
                        )),
                    )
                    .map(Some)
                }
            };
            let _ = sender.send(result);
        });
    }
}
impl eframe::App for Maintenance {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let result = self.pending.as_ref().and_then(|r| r.try_recv().ok());
        if let Some(result) = result {
            self.pending = None;
            match result {
                Ok(Some(ready)) => {
                    let target = self.target.take().unwrap();
                    // The launcher might have opened while the download was running.
                    let closed = if target.file_name().and_then(|n| n.to_str()) == Some(APP)
                        || self.recovery
                    {
                        ensure_closed(&target)
                    } else {
                        Ok(())
                    };
                    match closed.and_then(|()| updater::apply_to(&ready, &target)) {
                        Ok(()) => {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        Err(error) => self.status = format!("Update stopped: {error}"),
                    }
                }
                Ok(None) => self.status = "Already up to date.".into(),
                Err(error) => self.status = format!("{error:#}"),
            }
        }
        egui::CentralPanel::default().show(ctx,|ui| {
            ui.add_space(12.0);ui.heading(if self.recovery {"Canna Recovery"}else{"Canna Maintenance"});
            ui.label(format!("Updater {MAINTENANCE_VERSION}"));ui.add_space(8.0);
            ui.label("Update, repair or restore Canna. Close the launcher before making changes.");
            ui.add_space(10.0);
            ui.add_enabled_ui(self.pending.is_none(),|ui|{
                ui.horizontal(|ui| {
                    if ui.button("Update Canna").clicked(){self.start(Operation::AppUpdate);}
                    if ui.button("Repair Canna").clicked(){self.start(Operation::AppRepair);}
                    if ui.button("Restore previous Canna").clicked(){self.start(Operation::AppRestore);}
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Update updater").clicked(){self.start(Operation::UpdaterUpdate);}
                    if ui.button("Repair updater").clicked(){self.start(Operation::UpdaterRepair);}
                    if ui.button("Restore previous updater").clicked(){self.start(Operation::UpdaterRestore);}
                });
                ui.add_space(12.0);ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Uninstall Canna").clicked(){
                        let uninstaller=self.root.join("unins000.exe");
                        if uninstaller.is_file(){
                            match std::process::Command::new(uninstaller).spawn(){
                                Ok(_)=>ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                                Err(error)=>self.status=error.to_string(),
                            }
                        }else{self.status="This is a portable copy. Install Canna to use its full uninstaller.".into();}
                    }
                    if ui.button("Open update logs").clicked(){
                        let folder=std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("CannaModManager/updates");
                        if let Err(error)=fs::create_dir_all(&folder).and_then(|()|std::process::Command::new("explorer.exe").arg(folder).spawn().map(|_|())){self.status=error.to_string();}
                    }
                });
            });
            ui.add_space(16.0);if self.pending.is_some(){ui.spinner();ctx.request_repaint_after(std::time::Duration::from_millis(100));}
            ui.label(&self.status);ui.add_space(10.0);
            ui.small("Updates keep a verified previous copy. Canna Recovery.exe remains available if the updater cannot open.");
            ui.small(self.root.display().to_string());
        });
    }
}
fn main() -> eframe::Result {
    let exe = std::env::current_exe().expect("Maintenance executable path");
    let recovery = exe
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("Canna Recovery.exe"));
    let root = exe.parent().unwrap().to_owned();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([660.0, 340.0])
            .with_min_inner_size([640.0, 340.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/canna-logo.png"))
                    .unwrap(),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "Canna Maintenance",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut style = (*cc.egui_ctx.style()).clone();
            style.spacing.button_padding = egui::vec2(12.0, 9.0);
            style.visuals.panel_fill = egui::Color32::from_rgb(17, 28, 23);
            cc.egui_ctx.set_style(style);
            Ok(Box::new(Maintenance {
                root,
                recovery,
                status: if recovery {
                    "Recovery copy ready. Use Repair updater if its normal copy is missing or broken.".into()
                } else {
                    "Ready.".into()
                },
                pending: None,
                target: None,
            }))
        }),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Downloads and verifies the public maintenance release without applying it"]
    fn live_maintenance_release_is_verified() {
        let ready = updater::check_maintenance("0.0.0").unwrap().unwrap();
        assert!(fs::read(&ready.file).unwrap().starts_with(b"MZ"));
        fs::remove_file(ready.file).unwrap();
        assert!(updater::check_maintenance("999.0.0").unwrap().is_none());
    }
    #[test]
    fn rollback_is_verified_and_staged_before_replacement() {
        let root =
            std::env::temp_dir().join(format!("canna-maintenance-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let target = root.join(APP);
        let backup = target.with_extension("previous.exe");
        let bytes = b"MZ rollback fixture";
        fs::write(&backup, bytes).unwrap();
        fs::write(
            backup.with_extension("exe.sha256"),
            format!("{:x}", Sha256::digest(bytes)),
        )
        .unwrap();
        let staged = root.join("staged.exe");
        assert!(verified_rollback(&target, &staged).is_ok());
        assert_eq!(fs::read(staged).unwrap(), bytes);
        fs::write(&backup, b"MZ tampered").unwrap();
        assert!(verified_rollback(&target, &root.join("tampered-stage.exe")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
