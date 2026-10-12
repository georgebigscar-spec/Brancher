//! `git status --porcelain=v2 -z --branch`.

use brancher_core::{Change, ChangeKind, RepoStatus};

use super::{split_nul, utf8};
use crate::GitError;

pub fn args() -> [&'static str; 5] {
    [
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--untracked-files=all",
    ]
}

pub fn parse(out: &[u8]) -> Result<RepoStatus, GitError> {
    let mut status = RepoStatus::default();
    let mut entries = split_nul(out);
    while let Some(entry) = entries.next() {
        let line = utf8(entry);
        let bad = || GitError::Parse(format!("status: {line:?}"));
        if let Some(header) = line.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => status.head = Some(value.to_owned()),
                "branch.head" if value != "(detached)" => status.branch = Some(value.to_owned()),
                "branch.upstream" => status.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    let mut it = value.split(' ');
                    let a = it
                        .next()
                        .and_then(|s| s.strip_prefix('+'))
                        .and_then(|s| s.parse().ok());
                    let b = it
                        .next()
                        .and_then(|s| s.strip_prefix('-'))
                        .and_then(|s| s.parse().ok());
                    status.ahead = a.ok_or_else(bad)?;
                    status.behind = b.ok_or_else(bad)?;
                }
                _ => {}
            }
            continue;
        }
        let (tag, rest) = line.split_once(' ').ok_or_else(bad)?;
        let change = match tag {
            // 1 XY sub mH mI mW hH hI path
            "1" => {
                let f: Vec<&str> = rest.splitn(8, ' ').collect();
                let [xy, _, _, _, _, _, _, path] = f[..] else {
                    return Err(bad());
                };
                xy_change(xy, path, None).ok_or_else(bad)?
            }
            // 2 XY sub mH mI mW hH hI Xscore path NUL origPath
            "2" => {
                let f: Vec<&str> = rest.splitn(9, ' ').collect();
                let [xy, _, _, _, _, _, _, _, path] = f[..] else {
                    return Err(bad());
                };
                let orig = entries.next().map(utf8).ok_or_else(bad)?;
                xy_change(xy, path, Some(orig)).ok_or_else(bad)?
            }
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            "u" => {
                let f: Vec<&str> = rest.splitn(10, ' ').collect();
                let path = *f.get(9).ok_or_else(bad)?;
                Change {
                    path: path.to_owned(),
                    orig_path: None,
                    staged: Some(ChangeKind::Unmerged),
                    unstaged: Some(ChangeKind::Unmerged),
                }
            }
            "?" => Change {
                path: rest.to_owned(),
                orig_path: None,
                staged: None,
                unstaged: Some(ChangeKind::Untracked),
            },
            "!" => continue,
            _ => return Err(bad()),
        };
        status.changes.push(change);
    }
    Ok(status)
}

fn xy_change(xy: &str, path: &str, orig_path: Option<String>) -> Option<Change> {
    let mut c = xy.chars();
    let (x, y) = (c.next()?, c.next()?);
    Some(Change {
        path: path.to_owned(),
        orig_path,
        staged: ChangeKind::from_porcelain(x),
        unstaged: ChangeKind::from_porcelain(y),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_headers_and_entries() {
        let out = b"# branch.oid 1111111111111111111111111111111111111111\0\
# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -5\0\
1 .M N... 100644 100644 100644 aaa bbb src/main.rs\0\
1 A. N... 000000 100644 100644 000 ccc \xd0\xbf\xd1\x80\xd0\xb8\xd0\xb2\xd0\xb5\xd1\x82 \xd0\xbc\xd0\xb8\xd1\x80.txt\0\
2 R. N... 100644 100644 100644 ddd ddd R100 new name.rs\0old name.rs\0\
u UU N... 100644 100644 100644 100644 e1 e2 e3 conflict.txt\0\
? notes/todo.md\0! target/\0";
        let s = parse(out).unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (2, 5));
        assert_eq!(s.changes.len(), 5);

        assert_eq!(s.changes[0].path, "src/main.rs");
        assert_eq!(s.changes[0].staged, None);
        assert_eq!(s.changes[0].unstaged, Some(ChangeKind::Modified));

        // Путь с пробелом и не-ASCII символами (NFR-15).
        assert_eq!(s.changes[1].path, "привет мир.txt");
        assert_eq!(s.changes[1].staged, Some(ChangeKind::Added));

        assert_eq!(s.changes[2].path, "new name.rs");
        assert_eq!(s.changes[2].orig_path.as_deref(), Some("old name.rs"));
        assert_eq!(s.changes[2].staged, Some(ChangeKind::Renamed));

        assert!(s.changes[3].is_conflicted());
        assert_eq!(s.changes[4].unstaged, Some(ChangeKind::Untracked));

        assert_eq!(s.staged().count(), 2);
        assert_eq!(s.unstaged().count(), 3);
    }

    #[test]
    fn initial_and_detached() {
        let s = parse(b"# branch.oid (initial)\0# branch.head main\0").unwrap();
        assert_eq!(s.head, None);
        assert_eq!(s.branch.as_deref(), Some("main"));
        let s = parse(b"# branch.oid abc\0# branch.head (detached)\0").unwrap();
        assert_eq!(s.branch, None);
        assert_eq!(s.head.as_deref(), Some("abc"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"1 .M\0").is_err());
        assert!(parse(b"Z something\0").is_err());
    }
}
