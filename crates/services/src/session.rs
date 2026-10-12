//! Сессия между запусками: открытые вкладки, недавние репозитории и настройки.
//! Хранится в `state.json` в папке настроек ОС; запись — в фоне, атомарно (tmp + rename).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use brancher_core::{Session, Settings};
use serde::{Deserialize, Serialize};

/// Частые изменения (переключения вкладок) пишутся одной записью.
const DEBOUNCE: Duration = Duration::from_millis(300);
const VERSION: u32 = 1;

/// Формат файла. Отсутствующие поля берутся по умолчанию, лишние игнорируются,
/// чтобы файл от другой версии приложения не сбрасывал всё.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    version: u32,
    tabs: Vec<String>,
    active: Option<usize>,
    recent: Vec<String>,
    settings: StoredSettings,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct StoredSettings {
    side_by_side: bool,
    dark: bool,
}

/// Пути, которые нельзя записать строкой (не UTF-8), не сохраняются.
fn strings(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|p| p.to_str().map(str::to_owned))
        .collect()
}

pub fn encode(session: &Session) -> String {
    let stored = Stored {
        version: VERSION,
        tabs: strings(&session.tabs),
        active: session.active,
        recent: strings(&session.recent),
        settings: StoredSettings {
            side_by_side: session.settings.side_by_side,
            dark: session.settings.dark,
        },
    };
    serde_json::to_string_pretty(&stored).expect("сессия сериализуется")
}

pub fn decode(text: &str) -> Result<Session, serde_json::Error> {
    let s: Stored = serde_json::from_str(text)?;
    Ok(Session {
        tabs: s.tabs.into_iter().map(PathBuf::from).collect(),
        active: s.active,
        recent: s.recent.into_iter().map(PathBuf::from).collect(),
        settings: Settings {
            side_by_side: s.settings.side_by_side,
            dark: s.settings.dark,
        },
    })
}

/// `state.json` в папке настроек: `$BRANCHER_CONFIG_DIR`, иначе
/// `%APPDATA%\Brancher`, `~/Library/Application Support/Brancher` или
/// `$XDG_CONFIG_HOME/brancher` (`~/.config/brancher`).
pub fn default_path() -> Option<PathBuf> {
    let env = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let dir = if let Some(d) = env("BRANCHER_CONFIG_DIR") {
        d
    } else if cfg!(windows) {
        env("APPDATA")?.join("Brancher")
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support/Brancher")
    } else {
        env("XDG_CONFIG_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".config")))?
            .join("brancher")
    };
    Some(dir.join("state.json"))
}

/// Читает сессию; нет файла — пустая, повреждённый файл — пустая с сообщением в stderr.
pub fn load(path: &Path) -> Session {
    match std::fs::read_to_string(path) {
        Ok(text) => decode(&text).unwrap_or_else(|e| {
            eprintln!("не удалось прочитать {}: {e}", path.display());
            Session::default()
        }),
        Err(_) => Session::default(),
    }
}

fn write(path: &Path, session: &Session) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, encode(session))?;
    std::fs::rename(&tmp, path)
}

/// Фоновая запись сессии. При удалении дописывает последнее состояние.
pub struct SessionStore {
    tx: Option<Sender<Session>>,
    handle: Option<JoinHandle<()>>,
}

impl SessionStore {
    pub fn new(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel::<Session>();
        let handle = thread::Builder::new()
            .name("session".into())
            .spawn(move || {
                while let Ok(mut latest) = rx.recv() {
                    loop {
                        match rx.recv_timeout(DEBOUNCE) {
                            Ok(s) => latest = s,
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    if let Err(e) = write(&path, &latest) {
                        eprintln!("не удалось сохранить {}: {e}", path.display());
                    }
                }
            })
            .ok();
        Self { tx: Some(tx), handle }
    }

    pub fn save(&self, session: Session) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(session);
        }
    }
}

impl Drop for SessionStore {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        Session {
            tabs: vec!["/a".into(), "/путь/b".into()],
            active: Some(1),
            recent: vec!["/путь/b".into(), "/a".into(), "/c".into()],
            settings: Settings {
                side_by_side: true,
                dark: true,
            },
        }
    }

    #[test]
    fn encode_decode_round_trip() {
        assert_eq!(decode(&encode(&sample())).unwrap(), sample());
    }

    #[test]
    fn missing_and_unknown_fields_are_tolerated() {
        let s = decode(r#"{"tabs": ["/x"], "future": 1}"#).unwrap();
        assert_eq!(s.tabs, [PathBuf::from("/x")]);
        assert_eq!(s.settings, Settings::default());
        assert!(decode("not json").is_err());
    }

    #[test]
    fn store_writes_latest_on_drop_and_load_reads_it() {
        let dir = std::env::temp_dir().join(format!("brancher-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("state.json");
        assert_eq!(load(&path), Session::default());

        let store = SessionStore::new(path.clone());
        store.save(Session::default());
        store.save(sample());
        drop(store);
        assert_eq!(load(&path), sample());

        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(load(&path), Session::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
