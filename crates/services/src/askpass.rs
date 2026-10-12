//! Запросы логина и пароля от git и ssh (NFR-09).
//!
//! Brancher сам служит `GIT_ASKPASS` и `SSH_ASKPASS`: git запускает его копию с текстом
//! вопроса, копия (helper) подключается к приложению по localhost с токеном из окружения,
//! приложение показывает диалог, ответ уходит обратно в helper и из него — в git.
//!
//! Протокол: helper пишет `токен\nвопрос\n`; приложение отвечает `1\nответ\n` или `0\n` (отмена).

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::ffi::OsString;
use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use brancher_core::{JobResult, RepoId};

use crate::{Completed, Repaint};

/// Переменная окружения, по которой копия приложения понимает, что она helper:
/// `адрес токен`.
pub const ENV: &str = "BRANCHER_ASKPASS";
/// Сколько ждать ответа пользователя; потом git получает отказ.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(600);

type Waiting = Arc<Mutex<HashMap<u64, Sender<Option<String>>>>>;

/// Сервер запросов: живёт, пока живо приложение.
pub struct AskpassServer {
    addr: String,
    token: String,
    waiting: Waiting,
}

fn random_token() -> String {
    // Два независимых случайных ключа std — 128 бит, без зависимостей.
    let part = || RandomState::new().build_hasher().finish();
    format!("{:016x}{:016x}", part(), part())
}

impl AskpassServer {
    pub fn start(out: Sender<Completed>, repaint: Repaint) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?.to_string();
        let token = random_token();
        let waiting: Waiting = Arc::default();
        let next_id = Arc::new(AtomicU64::new(1));
        let (t, w) = (token.clone(), waiting.clone());
        thread::Builder::new().name("askpass".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                let (t, w, out, repaint, next_id) = (
                    t.clone(),
                    w.clone(),
                    out.clone(),
                    repaint.clone(),
                    next_id.clone(),
                );
                // Каждый запрос ждёт пользователя в своём потоке: git-процессов может быть несколько.
                let _ = thread::Builder::new()
                    .name("askpass-request".into())
                    .spawn(move || serve(stream, &t, &w, &out, &repaint, &next_id));
            }
        })?;
        Ok(Self { addr, token, waiting })
    }

    /// Переменные окружения для каждого запуска git.
    pub fn env(&self) -> Vec<(OsString, OsString)> {
        let Ok(exe) = std::env::current_exe() else {
            return vec![];
        };
        vec![
            ("GIT_ASKPASS".into(), exe.clone().into()),
            ("SSH_ASKPASS".into(), exe.into()),
            // OpenSSH 8.4+: спрашивать через SSH_ASKPASS и без терминала, и без DISPLAY.
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            (ENV.into(), format!("{} {}", self.addr, self.token).into()),
        ]
    }

    /// Ответ пользователя на запрос `id`; `None` — отмена.
    pub fn answer(&self, id: u64, answer: Option<String>) {
        if let Some(tx) = self.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
            let _ = tx.send(answer);
        }
    }
}

fn serve(
    stream: TcpStream,
    token: &str,
    waiting: &Waiting,
    out: &Sender<Completed>,
    repaint: &Repaint,
    next_id: &AtomicU64,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(&stream);
    let mut line = || {
        let mut s = String::new();
        reader
            .read_line(&mut s)
            .ok()
            .map(|_| s.trim_end_matches(['\r', '\n']).to_owned())
    };
    let (Some(got), Some(prompt)) = (line(), line()) else {
        return;
    };
    if got != token {
        return;
    }
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    waiting.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
    let _ = out.send((RepoId::APP, JobResult::Prompt { id, text: prompt }));
    repaint();
    let answer = rx.recv_timeout(ANSWER_TIMEOUT).ok().flatten();
    waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    let reply = match answer {
        Some(a) => format!("1\n{a}\n"),
        None => "0\n".to_owned(),
    };
    let _ = (&stream).write_all(reply.as_bytes());
}

/// Режим helper: `value` — содержимое [`ENV`], `prompt` — вопрос от git/ssh.
/// Возвращает ответ для stdout или `None` (отмена, приложение недоступно).
pub fn ask(value: &str, prompt: &str) -> Option<String> {
    let (addr, token) = value.split_once(' ')?;
    let mut stream = TcpStream::connect(addr).ok()?;
    let prompt = prompt.replace(['\r', '\n'], " ");
    stream.write_all(format!("{token}\n{prompt}\n").as_bytes()).ok()?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).ok()?;
    if status.trim() != "1" {
        return None;
    }
    let mut answer = String::new();
    reader.read_line(&mut answer).ok()?;
    Some(answer.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> (AskpassServer, mpsc::Receiver<Completed>, String) {
        let (tx, rx) = mpsc::channel();
        let s = AskpassServer::start(tx, Arc::new(|| {})).unwrap();
        let value = format!("{} {}", s.addr, s.token);
        (s, rx, value)
    }

    fn next_prompt(rx: &mpsc::Receiver<Completed>) -> (u64, String) {
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            (RepoId::APP, JobResult::Prompt { id, text }) => (id, text),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn helper_gets_user_answer_or_cancel() {
        let (s, rx, value) = server();
        let v = value.clone();
        let helper = thread::spawn(move || ask(&v, "Password for 'https://u@host': "));
        let (id, text) = next_prompt(&rx);
        assert_eq!(text, "Password for 'https://u@host': ");
        s.answer(id, Some("секрет 1".into()));
        assert_eq!(helper.join().unwrap().as_deref(), Some("секрет 1"));

        let v = value.clone();
        let helper = thread::spawn(move || ask(&v, "Username:"));
        let (id, _) = next_prompt(&rx);
        s.answer(id, None);
        assert_eq!(helper.join().unwrap(), None);
    }

    #[test]
    fn wrong_token_is_ignored() {
        let (_s, rx, value) = server();
        let addr = value.split_once(' ').unwrap().0.to_owned();
        assert_eq!(ask(&format!("{addr} wrong"), "Password:"), None);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        assert_eq!(ask("127.0.0.1:1 x", "Password:"), None);
    }

    #[test]
    fn env_points_git_and_ssh_to_the_app() {
        let (s, _rx, value) = server();
        let env: HashMap<_, _> = s.env().into_iter().collect();
        assert_eq!(env[&OsString::from(ENV)], OsString::from(value));
        assert_eq!(
            env[&OsString::from("GIT_ASKPASS")],
            env[&OsString::from("SSH_ASKPASS")]
        );
    }
}
