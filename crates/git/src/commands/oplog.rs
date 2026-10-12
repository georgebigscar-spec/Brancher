//! Лента отмены: перед изменяющей операцией Brancher сохраняет снимок репозитория —
//! ветки, теги, stash, HEAD и незакоммиченные изменения — коммитом в `refs/brancher/oplog`.
//!
//! Снимок — коммит с пустым деревом. Первый родитель — предыдущий снимок, остальные —
//! незакоммиченные изменения (`git stash create`) и все коммиты, на которые указывали
//! ссылки: так их не удалит `git gc`, даже если ветку потом удалят. Сами ссылки и HEAD
//! записаны в сообщении коммита.

use brancher_core::Snapshot;

use super::utf8;
use crate::GitError;

pub const REF: &str = "refs/brancher/oplog";
const PREFIX: &str = "Brancher: ";

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// Что лежит в снимке.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct State {
    /// `ref: refs/heads/x` или SHA detached HEAD; `None` — HEAD без коммитов.
    pub head: Option<String>,
    /// Коммит `git stash create` с индексом и рабочим деревом, если были изменения.
    pub wip: Option<String>,
    /// `(SHA, полное имя)` веток и тегов.
    pub refs: Vec<(String, String)>,
    /// Записи stash сверху вниз: `(SHA, сообщение)`.
    pub stashes: Vec<(String, String)>,
}

/// Ветки и теги; для аннотированного тега — ещё коммит, на который он указывает.
pub fn refs_args() -> Vec<String> {
    owned(&[
        "for-each-ref",
        "--format=%(objectname)%00%(objecttype)%00%(*objectname)%00%(*objecttype)%00%(refname)",
        "refs/heads",
        "refs/tags",
    ])
}

/// Ссылки и коммиты, которые нужно удержать от `git gc`.
pub fn parse_refs(out: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut refs = vec![];
    let mut commits = vec![];
    for line in out.lines() {
        let f: Vec<&str> = line.split('\0').collect();
        let [id, kind, peeled, peeled_kind, name] = f[..] else {
            continue;
        };
        refs.push((id.to_owned(), name.to_owned()));
        if kind == "commit" {
            commits.push(id.to_owned());
        } else if peeled_kind == "commit" {
            commits.push(peeled.to_owned());
        }
    }
    (refs, commits)
}

pub fn message(title: &str, state: &State) -> String {
    let mut m = format!("{PREFIX}{title}\n\n");
    m.push_str(&format!("head {}\n", state.head.as_deref().unwrap_or("-")));
    m.push_str(&format!("wip {}\n", state.wip.as_deref().unwrap_or("-")));
    for (id, name) in &state.refs {
        m.push_str(&format!("ref {id} {name}\n"));
    }
    for (id, msg) in &state.stashes {
        m.push_str(&format!("stash {id} {}\n", msg.replace('\n', " ")));
    }
    m
}

pub fn parse_message(msg: &str) -> Option<State> {
    let mut lines = msg.lines();
    lines.next()?.strip_prefix(PREFIX)?;
    let opt = |v: &str| (v != "-").then(|| v.to_owned());
    let mut state = State::default();
    for line in lines {
        let Some((key, rest)) = line.split_once(' ') else {
            continue;
        };
        match key {
            "head" => state.head = opt(rest),
            "wip" => state.wip = opt(rest),
            "ref" => {
                let (id, name) = rest.split_once(' ')?;
                state.refs.push((id.to_owned(), name.to_owned()));
            }
            "stash" => {
                let (id, m) = rest.split_once(' ').unwrap_or((rest, ""));
                state.stashes.push((id.to_owned(), m.to_owned()));
            }
            _ => {}
        }
    }
    Some(state)
}

/// Последние `limit` снимков, новые сверху.
pub fn list_args(limit: usize) -> Vec<String> {
    vec![
        "log".into(),
        "--first-parent".into(),
        format!("--max-count={limit}"),
        "--format=%H%x00%ct%x00%s%x1e".into(),
        REF.into(),
        "--".into(),
    ]
}

/// Первый снимок продолжается обычной историей (его первый родитель — коммит ветки),
/// поэтому список обрывается на первом коммите, который не снимок.
pub fn parse_list(out: &[u8]) -> Result<Vec<Snapshot>, GitError> {
    out.split(|b| *b == 0x1e)
        .map(|r| r.strip_prefix(b"\n").unwrap_or(r))
        .filter(|r| !r.is_empty() && *r != b"\n")
        .take_while(|rec| {
            rec.splitn(3, |b| *b == 0)
                .nth(2)
                .is_some_and(|subject| subject.starts_with(PREFIX.as_bytes()))
        })
        .map(|rec| {
            let f: Vec<String> = rec.split(|b| *b == 0).map(utf8).collect();
            let [id, time, subject] = &f[..] else {
                return Err(GitError::Parse(format!("oplog: {} полей вместо 3", f.len())));
            };
            Ok(Snapshot {
                id: id.clone(),
                time: time
                    .parse()
                    .map_err(|_| GitError::Parse(format!("oplog: время {time:?}")))?,
                title: subject[PREFIX.len()..].to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trip() {
        let state = State {
            head: Some("ref: refs/heads/main".into()),
            wip: None,
            refs: vec![
                ("aaa".into(), "refs/heads/main".into()),
                ("bbb".into(), "refs/tags/v1".into()),
            ],
            stashes: vec![("ccc".into(), "On main: две\nстроки".into())],
        };
        let m = message("Коммит «x»", &state);
        assert!(m.starts_with("Brancher: Коммит «x»\n\n"));
        let mut expected = state.clone();
        expected.stashes[0].1 = "On main: две строки".into();
        assert_eq!(parse_message(&m), Some(expected));
        assert_eq!(parse_message("обычный коммит"), None);
    }

    #[test]
    fn list_stops_at_regular_history() {
        let out = b"s2\x001\x00Brancher: b\x1e\ns1\x001\x00Brancher: a\x1e\nc\x001\x00init\x1e\n";
        let list = parse_list(out).unwrap();
        assert_eq!(
            list.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
    }

    #[test]
    fn refs_keep_commits_alive() {
        let out =
            "a\0commit\0\0\0refs/heads/main\nt\0tag\0c\0commit\0refs/tags/v1\nb\0blob\0\0\0refs/tags/blob\n";
        let (refs, commits) = parse_refs(out);
        assert_eq!(refs.len(), 3);
        assert_eq!(commits, ["a", "c"]);
    }
}
