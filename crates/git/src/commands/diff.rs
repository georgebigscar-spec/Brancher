//! `git diff` в формате unified diff и его парсер.

use brancher_core::{DiffLine, DiffTarget, FileDiff, Hunk, LineKind};

use super::utf8;
use crate::GitError;

/// Флаги, не зависящие от пользовательского конфига (`diff.noprefix`, внешние diff-утилиты).
const COMMON: [&str; 6] = [
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "-M",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

/// Аргументы для цели. Untracked-файл сравнивается с пустым через `--no-index`,
/// такой вызов завершается кодом 1, если файл не пустой (см. [`ok_codes`]).
pub fn args(target: &DiffTarget) -> Vec<String> {
    let mut a: Vec<String> = match target {
        DiffTarget::Commit { id, parent: Some(p) } => {
            let mut a = vec!["diff".into()];
            a.extend(COMMON.map(String::from));
            a.extend([p.clone(), id.clone(), "--".into()]);
            return a;
        }
        DiffTarget::Commit { id, parent: None } => {
            let mut a = vec![
                "diff-tree".into(),
                "-p".into(),
                "--root".into(),
                "--no-commit-id".into(),
            ];
            a.extend(COMMON.map(String::from));
            a.extend([id.clone(), "--".into()]);
            return a;
        }
        DiffTarget::Range { from, to } => {
            let mut a = vec!["diff".into()];
            a.extend(COMMON.map(String::from));
            a.extend([format!("{from}...{to}"), "--".into()]);
            return a;
        }
        DiffTarget::Change { untracked: true, .. } => vec!["diff".into(), "--no-index".into()],
        DiffTarget::Change { staged: true, .. } => vec!["diff".into(), "--cached".into()],
        DiffTarget::Change { conflicted: true, .. } => vec!["diff".into(), "HEAD".into()],
        DiffTarget::Change { .. } => vec!["diff".into()],
    };
    a.extend(COMMON.map(String::from));
    a.push("--".into());
    if let DiffTarget::Change { path, untracked, .. } = target {
        if *untracked {
            a.push("/dev/null".into());
        }
        a.push(path.clone());
    }
    a
}

/// Коды выхода, при которых вывод — нормальный diff.
pub fn ok_codes(target: &DiffTarget) -> &'static [i32] {
    match target {
        DiffTarget::Change { untracked: true, .. } => &[0, 1],
        _ => &[0],
    }
}

pub fn parse(out: &[u8]) -> Result<Vec<FileDiff>, GitError> {
    let mut files: Vec<FileDiff> = vec![];
    let mut old_no = 0u32;
    let mut new_no = 0u32;
    // Внутри ханка строки считаются по его заголовку, чтобы строка кода `--- x`
    // не приняли за заголовок файла.
    let mut left_old = 0u32;
    let mut left_new = 0u32;

    for raw in out.split(|b| *b == b'\n') {
        let cr = raw.ends_with(b"\r");
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let in_hunk = left_old > 0 || left_new > 0;
        if in_hunk || line.starts_with(b"\\") {
            let Some(file) = files.last_mut() else { continue };
            if file.hunks.is_empty() {
                continue;
            }
            let (kind, text) = match line.first() {
                Some(b'+') => (LineKind::Added, &line[1..]),
                Some(b'-') => (LineKind::Removed, &line[1..]),
                Some(b'\\') => (LineKind::NoNewline, line),
                Some(b' ') => (LineKind::Context, &line[1..]),
                // Пустая строка контекста, у которой редактор срезал пробел.
                None => (LineKind::Context, line),
                Some(_) => {
                    return Err(GitError::Parse(format!(
                        "diff: неожиданная строка в ханке {:?}",
                        utf8(line)
                    )))
                }
            };
            let (o, n) = match kind {
                LineKind::Context => (Some(old_no), Some(new_no)),
                LineKind::Removed => (Some(old_no), None),
                LineKind::Added => (None, Some(new_no)),
                LineKind::NoNewline => (None, None),
            };
            if o.is_some() {
                old_no += 1;
                left_old = left_old.saturating_sub(1);
            }
            if n.is_some() {
                new_no += 1;
                left_new = left_new.saturating_sub(1);
            }
            // Не-UTF-8 текст показывается с заменой символов, но patch из него
            // собрать нельзя: байты потеряны.
            file.lossy |= std::str::from_utf8(text).is_err();
            let hunk = file.hunks.last_mut().expect("проверено выше");
            hunk.lines.push(DiffLine {
                kind,
                old_no: o,
                new_no: n,
                text: utf8(text),
                cr: cr && kind != LineKind::NoNewline,
                spans: vec![],
            });
            continue;
        }

        if let Some(rest) = line.strip_prefix(b"diff --git ") {
            files.push(FileDiff::default());
            if let Some((a, b)) = split_git_header(&utf8(rest)) {
                let f = files.last_mut().expect("только что добавлен");
                f.old_path = Some(a);
                f.new_path = Some(b);
            }
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if line.starts_with(b"@@ ") {
            let header = utf8(line);
            let (os, oc, ns, nc) = parse_hunk_header(&header)
                .ok_or_else(|| GitError::Parse(format!("diff: заголовок ханка {header:?}")))?;
            (old_no, new_no, left_old, left_new) = (os, ns, oc, nc);
            file.hunks.push(Hunk {
                header,
                old_start: os,
                old_count: oc,
                new_start: ns,
                new_count: nc,
                lines: vec![],
            });
        } else if let Some(p) = line.strip_prefix(b"--- ") {
            file.old_path = side_path(p, b"a/");
        } else if let Some(p) = line.strip_prefix(b"+++ ") {
            file.new_path = side_path(p, b"b/");
        } else if let Some(m) = line.strip_prefix(b"new file mode ") {
            file.old_path = None;
            file.mode = Some(utf8(m));
        } else if let Some(m) = line.strip_prefix(b"deleted file mode ") {
            file.new_path = None;
            file.mode = Some(utf8(m));
        } else if let Some(p) = line.strip_prefix(b"rename from ") {
            file.old_path = Some(unquote(&utf8(p)));
        } else if let Some(p) = line.strip_prefix(b"rename to ") {
            file.new_path = Some(unquote(&utf8(p)));
        } else if line.starts_with(b"Binary files ") || line == b"GIT binary patch" {
            file.binary = true;
        }
    }
    Ok(files)
}

/// `--- a/path`, `+++ b/path` или `/dev/null`. После имени git может дописать табуляцию.
fn side_path(p: &[u8], prefix: &[u8]) -> Option<String> {
    let p = utf8(p);
    let p = p.strip_suffix('\t').unwrap_or(&p);
    if p == "/dev/null" {
        return None;
    }
    let p = unquote(p);
    Some(
        p.strip_prefix(std::str::from_utf8(prefix).unwrap_or_default())
            .unwrap_or(&p)
            .to_owned(),
    )
}

/// `a/x b/x`. Пути могут содержать пробелы, поэтому без кавычек берём совпадающие половины;
/// у переименований пути придут из `rename from/to` или `---`/`+++`.
fn split_git_header(rest: &str) -> Option<(String, String)> {
    if rest.starts_with('"') {
        let end = closing_quote(rest)?;
        let a = unquote(&rest[..=end]);
        let b = unquote(rest[end + 1..].trim_start());
        return Some((a.strip_prefix("a/")?.to_owned(), b.strip_prefix("b/")?.to_owned()));
    }
    let half = rest.len().checked_sub(1)? / 2;
    let (a, b) = (rest.get(..half)?, rest.get(half + 1..)?);
    match (a.strip_prefix("a/"), b.strip_prefix("b/")) {
        (Some(x), Some(y)) if x == y => Some((x.to_owned(), y.to_owned())),
        _ => None,
    }
}

fn closing_quote(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Снимает C-кавычки git (`"a\tb"`, `"\320\260"`). Строка без кавычек возвращается как есть.
fn unquote(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
        return s.to_owned();
    };
    let mut out = Vec::with_capacity(inner.len());
    let mut it = inner.bytes().peekable();
    while let Some(c) = it.next() {
        if c != b'\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some(b'n') => out.push(b'\n'),
            Some(b't') => out.push(b'\t'),
            Some(b'r') => out.push(b'\r'),
            Some(b'a') => out.push(7),
            Some(b'b') => out.push(8),
            Some(b'f') => out.push(12),
            Some(b'v') => out.push(11),
            Some(d @ b'0'..=b'7') => {
                let mut v = u32::from(d - b'0');
                for _ in 0..2 {
                    match it.peek() {
                        Some(&d @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(d - b'0');
                            it.next();
                        }
                        _ => break,
                    }
                }
                out.push(v as u8);
            }
            Some(other) => out.push(other),
            None => out.push(b'\\'),
        }
    }
    utf8(&out)
}

/// `@@ -1,5 +1,6 @@ fn main()` → (1, 5, 1, 6). Пропущенное число строк означает 1.
fn parse_hunk_header(h: &str) -> Option<(u32, u32, u32, u32)> {
    let mut parts = h.strip_prefix("@@ ")?.split(' ');
    let range = |s: &str| -> Option<(u32, u32)> {
        let (start, count) = s.split_once(',').unwrap_or((s, "1"));
        Some((start.parse().ok()?, count.parse().ok()?))
    };
    let (os, oc) = range(parts.next()?.strip_prefix('-')?)?;
    let (ns, nc) = range(parts.next()?.strip_prefix('+')?)?;
    Some((os, oc, ns, nc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,4 +1,5 @@ fn main() {
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
+    -- not a header
 }

diff --git a/new file.txt b/new file.txt
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/new file.txt
@@ -0,0 +1 @@
+hello
\\ No newline at end of file
diff --git a/old.txt b/renamed.txt
similarity index 90%
rename from old.txt
rename to renamed.txt
diff --git a/img.png b/img.png
index 4444444..5555555 100644
Binary files a/img.png and b/img.png differ
";

    #[test]
    fn parses_files_hunks_and_line_numbers() {
        let files = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(files.len(), 4);

        let main = &files[0];
        assert_eq!(main.path(), "src/main.rs");
        let h = &main.hunks[0];
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count), (1, 4, 1, 5));
        let kinds: Vec<_> = h.lines.iter().map(|l| l.kind).collect();
        use LineKind::*;
        assert_eq!(kinds, [Context, Removed, Added, Added, Context, Context]);
        assert_eq!(h.lines[3].text, "    -- not a header");
        assert_eq!((h.lines[1].old_no, h.lines[1].new_no), (Some(2), None));
        assert_eq!((h.lines[3].old_no, h.lines[3].new_no), (None, Some(3)));
        assert_eq!((h.lines[5].old_no, h.lines[5].new_no), (Some(4), Some(5)));
        assert_eq!(main.stats(), (2, 1));

        let added = &files[1];
        assert_eq!((added.old_path.as_deref(), added.path()), (None, "new file.txt"));
        assert_eq!(added.hunks[0].lines.last().unwrap().kind, NoNewline);

        assert!(files[2].is_rename());
        assert_eq!(files[2].old_path.as_deref(), Some("old.txt"));
        assert_eq!(files[2].path(), "renamed.txt");
        assert!(files[2].hunks.is_empty());

        assert!(files[3].binary);
        assert_eq!(files[3].path(), "img.png");
    }

    #[test]
    fn deleted_file_keeps_old_path() {
        let out = "diff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let f = &parse(out.as_bytes()).unwrap()[0];
        assert_eq!(
            (f.old_path.as_deref(), f.new_path.as_deref()),
            (Some("gone.txt"), None)
        );
        assert_eq!(f.stats(), (0, 1));
    }

    #[test]
    fn quoted_paths_are_unquoted() {
        let out = "diff --git \"a/tab\\there.txt\" \"b/tab\\there.txt\"\nnew file mode 100644\n";
        let f = &parse(out.as_bytes()).unwrap()[0];
        assert_eq!(f.path(), "tab\there.txt");
        assert_eq!(unquote("\"\\320\\260\""), "а");
    }

    #[test]
    fn hunk_header_variants() {
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), Some((1, 1, 1, 1)));
        assert_eq!(parse_hunk_header("@@ -0,0 +1,3 @@ x"), Some((0, 0, 1, 3)));
        assert_eq!(parse_hunk_header("@@ broken"), None);
    }

    #[test]
    fn args_per_target() {
        let commit = DiffTarget::Commit {
            id: "b".into(),
            parent: Some("a".into()),
        };
        assert_eq!(args(&commit)[0], "diff");
        assert!(args(&commit).ends_with(&["a".into(), "b".into(), "--".into()]));
        let root = DiffTarget::Commit {
            id: "a".into(),
            parent: None,
        };
        assert_eq!(args(&root)[..3], ["diff-tree", "-p", "--root"]);
        let untracked = DiffTarget::Change {
            path: "-x".into(),
            staged: false,
            untracked: true,
            conflicted: false,
        };
        assert!(args(&untracked).ends_with(&["--".into(), "/dev/null".into(), "-x".into()]));
        assert_eq!(ok_codes(&untracked), [0, 1]);
        let staged = DiffTarget::Change {
            path: "x".into(),
            staged: true,
            untracked: false,
            conflicted: false,
        };
        assert_eq!(args(&staged)[..2], ["diff", "--cached"]);
    }
}
