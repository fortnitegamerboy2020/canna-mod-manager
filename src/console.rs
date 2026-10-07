use crate::model::InstalledGame;
use eframe::egui::{self, RichText};
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

const MAX_LOG_BYTES: u64 = 256 * 1024;
pub struct LogFile {
    pub path: Option<PathBuf>,
    pub text: String,
    pub modified: Option<SystemTime>,
}
pub struct Snapshot {
    pub files: Vec<LogFile>,
    pub running: Result<bool, String>,
}
pub struct Console {
    pub game_id: u32,
    source: usize,
    snapshots: std::collections::BTreeMap<u32, Snapshot>,
    manager: VecDeque<String>,
    started: Instant,
    query: String,
    errors_only: bool,
    follow: bool,
}
impl Console {
    pub fn new() -> Self {
        Self {
            game_id: 1686940,
            source: 1,
            snapshots: Default::default(),
            manager: Default::default(),
            started: Instant::now(),
            query: String::new(),
            errors_only: false,
            follow: true,
        }
    }
    pub fn record(&mut self, message: &str, token: &str) {
        let clean = if token.is_empty() {
            message.to_owned()
        } else {
            message.replace(token, "[redacted]")
        };
        self.manager
            .push_back(format!("[+{}s] {clean}", self.started.elapsed().as_secs()));
        while self.manager.len() > 1000 {
            self.manager.pop_front();
        }
    }
    pub fn update(&mut self, id: u32, snapshot: Snapshot) {
        self.snapshots.insert(id, snapshot);
    }
    pub fn has_snapshot(&self) -> bool {
        self.snapshots.contains_key(&self.game_id)
    }
    pub fn show(&mut self, ui: &mut egui::Ui, games: &[InstalledGame]) -> bool {
        ui.heading("Console");
        ui.label("Live setup, launch and game logs");
        ui.add_space(12.0);
        let mut refresh = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("console_game")
                .selected_text(
                    games
                        .iter()
                        .find(|g| g.app_id == self.game_id)
                        .map(|g| g.name.as_str())
                        .unwrap_or("Choose a game"),
                )
                .show_ui(ui, |ui| {
                    for game in games {
                        if ui
                            .selectable_value(&mut self.game_id, game.app_id, &game.name)
                            .changed()
                        {
                            refresh = true;
                        }
                    }
                });
            if ui.button("Refresh").clicked() {
                refresh = true;
            }
            ui.checkbox(&mut self.follow, "Follow output");
            ui.checkbox(&mut self.errors_only, "Errors & warnings");
        });
        let source_game = crate::model::source_addons(self.game_id).is_some();
        let sources: &[&str] = if source_game {
            &["Canna", "Source game"]
        } else {
            &["Canna", "BepInEx", "Unity", "Preloader"]
        };
        if self.source >= sources.len() {
            self.source = 1;
        }
        ui.horizontal_wrapped(|ui| {
            for (id, name) in sources.iter().enumerate() {
                ui.selectable_value(&mut self.source, id, *name);
            }
        });
        let file = self.snapshots.get(&self.game_id).and_then(|snapshot| {
            self.source
                .checked_sub(1)
                .and_then(|i| snapshot.files.get(i))
        });
        let text = if self.source == 0 {
            self.manager.iter().cloned().collect::<Vec<_>>().join("\n")
        } else {
            file.map(|f| f.text.clone())
                .unwrap_or_else(|| "Waiting for log refresh…".into())
        };
        if let Some(file) = file {
            if let Some(path) = &file.path {
                ui.label(RichText::new(path.display().to_string()).small());
                ui.horizontal(|ui| {
                    if let Some(modified) = file.modified {
                        ui.label(format!(
                            "File updated {} seconds ago",
                            modified.elapsed().unwrap_or_default().as_secs()
                        ));
                    }
                    if ui.button("Open log folder").clicked() {
                        let _ = std::process::Command::new("explorer.exe")
                            .arg(path.parent().unwrap_or(path))
                            .spawn();
                    }
                });
            }
            ui.label(
                RichText::new(
                    "Logs remain after the game closes; timestamps distinguish earlier runs.",
                )
                .small(),
            );
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Filter output…"));
            if ui.button("Copy output").clicked() {
                ui.ctx().copy_text(text.clone());
            }
        });
        ui.separator();
        egui::ScrollArea::both()
            .stick_to_bottom(self.follow)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in text.lines().filter(|line| {
                    let lower = line.to_lowercase();
                    lower.contains(&self.query.to_lowercase())
                        && (!self.errors_only
                            || lower.contains("error")
                            || lower.contains("exception")
                            || lower.contains("warn")
                            || lower.contains("failed"))
                }) {
                    let lower = line.to_lowercase();
                    let color = if lower.contains("error") || lower.contains("exception") {
                        egui::Color32::from_rgb(239, 143, 143)
                    } else if lower.contains("warn") {
                        egui::Color32::from_rgb(230, 196, 125)
                    } else {
                        egui::Color32::from_rgb(209, 226, 213)
                    };
                    ui.label(RichText::new(line).monospace().color(color));
                }
            });
        refresh
    }
}
fn tail(path: &Path) -> LogFile {
    let modified = fs::metadata(path).and_then(|m| m.modified()).ok();
    let result = (|| -> std::io::Result<String> {
        let mut file = fs::File::open(path)?;
        let len = file.metadata()?.len();
        let offset = len.saturating_sub(MAX_LOG_BYTES);
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(MAX_LOG_BYTES).read_to_end(&mut bytes)?;
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if offset > 0 {
            if let Some(end) = text.find('\n') {
                text.drain(..=end);
            }
            text = format!("[Showing the last 256 KiB]\n{text}");
        }
        Ok(text)
    })();
    LogFile {
        path: Some(path.to_owned()),
        modified,
        text: match result {
            Ok(text) if text.is_empty() => "Log file is empty. Waiting for output…".into(),
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                "No log yet. Launch the game to generate output.".into()
            }
            Err(error) => format!("Cannot read log: {error}"),
        },
    }
}
pub fn collect(game: &InstalledGame) -> Snapshot {
    if game.app_id == u32::MAX {
        return Snapshot {
            files: vec![tail(&game.path.join("logs/latest.log"))],
            running: crate::runtime::game_running(game).map_err(|e| e.to_string()),
        };
    }
    if let Some(addons) = crate::model::source_addons(game.app_id) {
        let content = game.path.join(addons).parent().unwrap().to_owned();
        let mut file = tail(&content.join("console.log"));
        if !content.join("console.log").exists() {
            file.text = "No Source console log yet. Launch this game through Canna to enable logging. An already-running game must be restarted through Canna.".into();
        }
        return Snapshot {
            files: vec![file],
            running: crate::runtime::game_running(game).map_err(|e| e.to_string()),
        };
    }
    let bepinex = [
        game.path.join("BepInEx/LogOutput.log"),
        game.path.join("BepInEx/LogOutput.txt"),
    ]
    .into_iter()
    .max_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok())
    .unwrap();
    let unity = if game.app_id == 1686940 {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("AppData/LocalLow/Johan Gronvall/BoplBattle/Player.log")
    } else {
        game.path.join("output_log.txt")
    };
    let preloader = fs::read_dir(&game.path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .take(200)
        .map(|e| e.path())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .to_lowercase()
                    .starts_with("preloader")
            })
        })
        .max_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok());
    Snapshot {
        files: vec![
            tail(&bepinex),
            tail(&unity),
            preloader.as_deref().map(tail).unwrap_or(LogFile {
                path: None,
                modified: None,
                text: "No preloader error log found.".into(),
            }),
        ],
        running: crate::runtime::game_running(game).map_err(|e| e.to_string()),
    }
}
pub struct LaunchWatch {
    pub game_id: u32,
    pub modded: bool,
    pub requested: SystemTime,
    started: Instant,
    was_running: bool,
    loader_seen: bool,
}
impl LaunchWatch {
    pub fn new(id: u32, modded: bool, requested: SystemTime) -> Self {
        Self {
            game_id: id,
            modded,
            requested,
            started: Instant::now(),
            was_running: false,
            loader_seen: false,
        }
    }
    pub fn update(&mut self, snapshot: &Snapshot) -> (String, bool) {
        let running = match snapshot.running {
            Ok(value) => value,
            Err(_) => {
                return (
                    "Launch requested; process status unavailable. See Console.".into(),
                    true,
                );
            }
        };
        if running {
            self.was_running = true;
            let source_game = crate::model::source_addons(self.game_id).is_some();
            self.loader_seen |= snapshot.files.iter().any(|file| {
                file.modified.is_some_and(|time| time >= self.requested)
                    && if source_game {
                        !file.text.is_empty()
                            && !file.text.starts_with("Log file is empty")
                            && !file.text.starts_with("Cannot read log")
                    } else {
                        file.text.contains("Chainloader startup complete")
                    }
            });
            return (
                if !self.modded {
                    "Game running — vanilla mode"
                } else if source_game && self.loader_seen {
                    "Game running — Source console output received"
                } else if source_game {
                    "Game running — waiting for Source console output; see Console"
                } else if self.loader_seen {
                    "Game running — BepInEx startup confirmed"
                } else {
                    "Game running — waiting for BepInEx output; see Console"
                }
                .into(),
                false,
            );
        }
        if self.was_running {
            return ("Game closed".into(), true);
        }
        if self.started.elapsed().as_secs() >= 30 {
            return (
                "Steam launch requested, but the game process was not detected. See Console."
                    .into(),
                true,
            );
        }
        (
            "Steam launch requested — waiting for game process…".into(),
            false,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_console_reads_engine_log_and_rejects_stale_launch_output() {
        let root =
            std::env::temp_dir().join(format!("canna-source-console-{}", std::process::id()));
        fs::create_dir_all(root.join("left4dead2")).unwrap();
        let game = InstalledGame {
            app_id: 550,
            name: "Left 4 Dead 2".into(),
            path: root.clone(),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        fs::write(
            root.join("left4dead2/console.log"),
            "Loading server plugin
[Canna Auto-Hop] bootstrap ready
",
        )
        .unwrap();
        let mut snapshot = collect(&game);
        assert_eq!(snapshot.files.len(), 1);
        assert!(snapshot.files[0].text.contains("bootstrap ready"));
        snapshot.running = Ok(true);
        let now = SystemTime::now();
        let mut watch = LaunchWatch::new(550, true, now);
        snapshot.files[0].modified = Some(now - std::time::Duration::from_secs(10));
        assert!(watch.update(&snapshot).0.contains("waiting for Source"));
        snapshot.files[0].modified = Some(now);
        assert!(
            watch
                .update(&snapshot)
                .0
                .contains("Source console output received")
        );
        fs::remove_file(root.join("left4dead2/console.log")).unwrap();
        assert!(
            collect(&game).files[0]
                .text
                .contains("Launch this game through Canna")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn launch_status_tracks_process_exit_and_rejects_old_logs() {
        let now = SystemTime::now();
        let mut watch = LaunchWatch::new(1686940, true, now);
        let mut snapshot = Snapshot {
            running: Ok(true),
            files: vec![LogFile {
                path: None,
                text: "Chainloader startup complete".into(),
                modified: Some(now - std::time::Duration::from_secs(1)),
            }],
        };
        assert!(watch.update(&snapshot).0.contains("waiting for BepInEx"));
        snapshot.files[0].modified = Some(now);
        assert!(watch.update(&snapshot).0.contains("confirmed"));
        snapshot.running = Ok(false);
        assert_eq!(watch.update(&snapshot), ("Game closed".into(), true));
    }
    #[test]
    fn log_tail_is_bounded_and_manager_redacts_token() {
        let path = std::env::temp_dir().join(format!("canna-console-{}.log", std::process::id()));
        fs::write(&path, "line\n".repeat(100000)).unwrap();
        let output = tail(&path);
        assert!(output.text.len() < MAX_LOG_BYTES as usize + 100);
        assert!(output.text.starts_with("[Showing"));
        fs::remove_file(path).unwrap();
        let mut console = Console::new();
        console.record("bad secret-token-value", "secret-token-value");
        assert!(!console.manager[0].contains("secret-token-value"));
    }
}
