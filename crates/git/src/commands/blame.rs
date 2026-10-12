//! `git blame --porcelain`: автор и коммит каждой строки файла.

use std::collections::HashMap;

use brancher_core::{Blame, BlameCommit};

use super::utf8;
use crate::GitError;

/// `rev` — коммит; `None` — рабочая копия (её новые строки принадлежат нулевому коммиту).
pub fn args(path: &str, rev: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = vec!["blame".into(), "--porcelain".into()];
    a.extend(rev.map(str::to_owned));
    a.extend(["--".into(), path.into()]);
    a
}

/// В porcelain-выводе сведения о коммите идут только при первой его строке:
/// `<sha> <было> <стало> [<число>]`, заголовки `ключ значение`, затем `\t<текст строки>`.
pub fn parse(out: &[u8]) -> Result<Blame, GitError> {
    let mut blame = Blame::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut current: Option<usize> = None;
    for raw in out.split(|b| *b == b'\n') {
        if let Some(text) = raw.strip_prefix(b"\t") {
            let c = current.ok_or_else(|| GitError::Parse("blame: строка без коммита".into()))?;
            blame.lines.push((c, utf8(text)));
            current = None;
            continue;
        }
        let line = utf8(raw);
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        match current {
            None if key.len() >= 40 && key.bytes().all(|b| b.is_ascii_hexdigit()) => {
                let next = blame.commits.len();
                let i = *index.entry(key.to_owned()).or_insert(next);
                if i == next {
                    blame.commits.push(BlameCommit {
                        id: key.to_owned(),
                        ..Default::default()
                    });
                }
                current = Some(i);
            }
            Some(i) => {
                let c = &mut blame.commits[i];
                match key {
                    "author" => c.author = value.to_owned(),
                    "author-time" => c.time = value.parse().unwrap_or_default(),
                    "summary" => c.summary = value.to_owned(),
                    _ => {}
                }
            }
            None => return Err(GitError::Parse(format!("blame: {line:?}"))),
        }
    }
    Ok(blame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let a = "a".repeat(40);
        let z = "0".repeat(40);
        let out = format!(
            "{a} 1 1 2\nauthor Анна\nauthor-time 100\nsummary init\nfilename f\n\tone\n\
             {a} 2 2\n\ttwo\n\
             {z} 3 3 1\nauthor Not Committed Yet\nauthor-time 200\nsummary Version of f from f\nfilename f\n\tthree\n"
        );
        let b = parse(out.as_bytes()).unwrap();
        assert_eq!(b.commits.len(), 2);
        assert_eq!(b.commits[0].author, "Анна");
        assert_eq!(b.commits[0].summary, "init");
        assert!(b.commits[1].is_uncommitted());
        let lines: Vec<_> = b.lines.iter().map(|(c, t)| (*c, t.as_str())).collect();
        assert_eq!(lines, [(0, "one"), (0, "two"), (1, "three")]);
        assert!(parse(b"").unwrap().lines.is_empty());
    }

    #[test]
    fn blame_args() {
        assert_eq!(args("a.rs", None), ["blame", "--porcelain", "--", "a.rs"]);
        assert_eq!(
            args("a.rs", Some("abc")),
            ["blame", "--porcelain", "abc", "--", "a.rs"]
        );
    }
}
