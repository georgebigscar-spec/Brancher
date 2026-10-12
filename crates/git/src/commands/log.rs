//! `git log` с машинным форматом: поля через NUL, записи через RS (0x1e).

use brancher_core::{Commit, Signature};

use super::utf8;
use crate::GitError;

const FORMAT: &str = "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%cn%x00%ce%x00%ct%x00%s%x1e";

pub fn args(limit: usize) -> Vec<String> {
    vec![
        "log".into(),
        // Коммиты stash показываются отдельным списком, а не в графе.
        "--exclude=refs/stash".into(),
        "--all".into(),
        "--date-order".into(),
        "--no-color".into(),
        format!("--max-count={limit}"),
        FORMAT.into(),
    ]
}

/// История одного файла с переименованиями (`--follow`), новые сверху.
pub fn file_args(path: &str, limit: usize) -> Vec<String> {
    vec![
        "log".into(),
        "--follow".into(),
        "--no-color".into(),
        format!("--max-count={limit}"),
        FORMAT.into(),
        "--".into(),
        path.into(),
    ]
}

pub fn parse(out: &[u8]) -> Result<Vec<Commit>, GitError> {
    out.split(|b| *b == 0x1e)
        .map(|r| r.strip_prefix(b"\n").unwrap_or(r))
        .filter(|r| !r.is_empty() && *r != b"\n")
        .map(parse_record)
        .collect()
}

fn parse_record(rec: &[u8]) -> Result<Commit, GitError> {
    let f: Vec<String> = rec.split(|b| *b == 0).map(utf8).collect();
    let [id, parents, an, ae, at, cn, ce, ct, subject] = &f[..] else {
        return Err(GitError::Parse(format!("log: {} полей вместо 9", f.len())));
    };
    let time = |s: &str| {
        s.parse::<i64>()
            .map_err(|_| GitError::Parse(format!("log: время {s:?}")))
    };
    Ok(Commit {
        id: id.clone(),
        parents: parents.split_whitespace().map(str::to_owned).collect(),
        author: Signature {
            name: an.clone(),
            email: ae.clone(),
            time: time(at)?,
        },
        committer: Signature {
            name: cn.clone(),
            email: ce.clone(),
            time: time(ct)?,
        },
        subject: subject.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records_including_merge_and_root() {
        let rec = |f: [&str; 9]| format!("{}\x1e\n", f.join("\0"));
        let out = rec([
            "bbbb",
            "aaaa cccc",
            "Анна",
            "anna@example.com",
            "1700000100",
            "Анна",
            "anna@example.com",
            "1700000100",
            "Merge branch 'feature'",
        ]) + &rec([
            "aaaa",
            "",
            "Bob",
            "bob@example.com",
            "1700000000",
            "Bob",
            "bob@example.com",
            "1700000001",
            "Initial commit",
        ]);
        let log = parse(out.as_bytes()).unwrap();
        assert_eq!(log.len(), 2);
        assert!(log[0].is_merge());
        assert_eq!(log[0].author.name, "Анна");
        assert_eq!(log[1].parents, Vec::<String>::new());
        assert_eq!(log[1].committer.time, 1700000001);
        assert_eq!(log[1].subject, "Initial commit");
    }

    #[test]
    fn empty_output_is_empty_history() {
        assert!(parse(b"").unwrap().is_empty());
    }

    #[test]
    fn args_contain_limit() {
        assert!(args(500).contains(&"--max-count=500".to_owned()));
    }
}
