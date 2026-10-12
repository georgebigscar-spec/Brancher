//! `git for-each-ref` — локальные и удалённые ветки, теги.

use brancher_core::{GitRef, RefKind};

use super::utf8;
use crate::GitError;

pub fn args() -> [&'static str; 6] {
    [
        "for-each-ref",
        "--format=%(refname)%00%(objectname)%00%(*objectname)%00%(HEAD)%00%(upstream)%00%(upstream:track,nobracket)%00",
        "--sort=refname",
        "refs/heads",
        "refs/remotes",
        "refs/tags",
    ]
}

/// Каждая ссылка — строка из 6 полей, оканчивающихся NUL, затем перевод строки.
pub fn parse(out: &[u8]) -> Result<Vec<GitRef>, GitError> {
    let mut refs = Vec::new();
    for line in out.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let f: Vec<String> = split_fields(line);
        let [full, oid, peeled, head, upstream, track] = &f[..] else {
            return Err(GitError::Parse(format!("for-each-ref: {:?}", utf8(line))));
        };
        let (kind, name) = if let Some(n) = full.strip_prefix("refs/heads/") {
            (RefKind::LocalBranch, n)
        } else if let Some(n) = full.strip_prefix("refs/remotes/") {
            if n.ends_with("/HEAD") {
                continue; // origin/HEAD — символьная ссылка, не ветка
            }
            (RefKind::RemoteBranch, n)
        } else if let Some(n) = full.strip_prefix("refs/tags/") {
            (RefKind::Tag, n)
        } else {
            continue;
        };
        let (ahead, behind, gone) = parse_track(track);
        refs.push(GitRef {
            name: name.to_owned(),
            full_name: full.clone(),
            kind,
            // Для аннотированного тега берём коммит, на который он указывает.
            target: if peeled.is_empty() {
                oid.clone()
            } else {
                peeled.clone()
            },
            is_head: head == "*",
            upstream: (!upstream.is_empty()).then(|| upstream.clone()),
            ahead,
            behind,
            upstream_gone: gone,
            merged: false,
        });
    }
    Ok(refs)
}

/// Основная ветка репозитория: локальная `main`, иначе `master`.
pub fn trunk(refs: &[GitRef]) -> Option<&GitRef> {
    ["main", "master"].iter().find_map(|n| {
        refs.iter()
            .find(|r| r.kind == RefKind::LocalBranch && r.name == *n)
    })
}

/// Локальные ветки, достижимые из `trunk` (полное имя).
pub fn merged_args(trunk: &str) -> Vec<String> {
    vec![
        "for-each-ref".into(),
        format!("--merged={trunk}"),
        "--format=%(refname)".into(),
        "refs/heads".into(),
    ]
}

/// Отмечает слитые ветки по выводу [`merged_args`]. Сама основная ветка и ветки на её
/// вершине (только что созданные, без своих коммитов) слитыми не считаются.
pub fn mark_merged(refs: &mut [GitRef], out: &[u8]) {
    let Some(trunk) = trunk(refs).map(|t| (t.full_name.clone(), t.target.clone())) else {
        return;
    };
    let text = utf8(out);
    let merged: Vec<&str> = text.lines().collect();
    for r in refs.iter_mut().filter(|r| r.kind == RefKind::LocalBranch) {
        r.merged = r.full_name != trunk.0 && r.target != trunk.1 && merged.contains(&r.full_name.as_str());
    }
}

fn split_fields(line: &[u8]) -> Vec<String> {
    // Поля могут быть пустыми, поэтому общий split_nul (отбрасывающий пустые) здесь не подходит.
    let line = line.strip_suffix(b"\0").unwrap_or(line);
    line.split(|b| *b == 0).map(utf8).collect()
}

/// `ahead 2, behind 5` | `ahead 1` | `behind 3` | `gone` | ``.
fn parse_track(s: &str) -> (u32, u32, bool) {
    if s == "gone" {
        return (0, 0, true);
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in s.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_merged_branches_except_trunk_and_fresh_ones() {
        let r = |name: &str, target: &str| GitRef {
            name: name.into(),
            full_name: format!("refs/heads/{name}"),
            kind: RefKind::LocalBranch,
            target: target.into(),
            is_head: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            merged: false,
        };
        let mut refs = vec![r("done", "a"), r("fresh", "t"), r("main", "t"), r("wip", "w")];
        assert_eq!(trunk(&refs).unwrap().name, "main");
        assert_eq!(merged_args("refs/heads/main")[1], "--merged=refs/heads/main");
        mark_merged(&mut refs, b"refs/heads/done\nrefs/heads/fresh\nrefs/heads/main\n");
        let merged: Vec<_> = refs
            .iter()
            .filter(|r| r.merged)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(merged, ["done"]);
        let mut no_trunk = vec![r("x", "a")];
        mark_merged(&mut no_trunk, b"refs/heads/x\n");
        assert!(!no_trunk[0].merged);
    }

    #[test]
    fn parses_branches_remotes_and_tags() {
        let out = b"refs/heads/main\0aaa\0\0*\0refs/remotes/origin/main\0ahead 2, behind 5\0\n\
refs/heads/old\0bbb\0\0 \0refs/remotes/origin/old\0gone\0\n\
refs/remotes/origin/HEAD\0aaa\0\0 \0\0\0\n\
refs/remotes/origin/main\0ccc\0\0 \0\0\0\n\
refs/tags/v1.0\0ddd\0eee\0 \0\0\0\n";
        let r = parse(out).unwrap();
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].name, "main");
        assert!(r[0].is_head);
        assert_eq!((r[0].ahead, r[0].behind), (2, 5));
        assert_eq!(r[0].upstream.as_deref(), Some("refs/remotes/origin/main"));
        assert!(r[1].upstream_gone);
        assert_eq!(r[2].kind, RefKind::RemoteBranch);
        assert_eq!(r[2].name, "origin/main");
        assert_eq!(r[3].kind, RefKind::Tag);
        assert_eq!(r[3].target, "eee");
    }
}
