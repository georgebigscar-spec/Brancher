//! Absorb: раскладка проиндексированных фрагментов по коммитам ветки, которые последними
//! меняли эти строки.

/// Diff индекса без контекста: каждый фрагмент можно применить отдельно.
pub fn staged_diff_args() -> Vec<String> {
    [
        "diff",
        "--cached",
        "-U0",
        "--no-color",
        "--no-ext-diff",
        "--no-renames",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ]
    .map(String::from)
    .to_vec()
}

pub fn branch_refs_args() -> Vec<String> {
    [
        "for-each-ref",
        "--format=%(refname)",
        "refs/heads",
        "refs/remotes",
    ]
    .map(String::from)
    .to_vec()
}

/// Коммиты текущей ветки от новых к старым: `%H NUL %P NUL %s NUL`. Ревизии — в stdin,
/// см. [`candidates_input`].
pub fn candidates_args(limit: usize) -> Vec<String> {
    let mut args: Vec<String> = ["log", "--first-parent", "--format=%H%x00%P%x00%s%x00"]
        .map(String::from)
        .to_vec();
    args.push(format!("-n{limit}"));
    args.push("--stdin".into());
    args
}

/// HEAD без коммитов, которые есть в других ветках (локальных и remote): их не переписываем.
pub fn candidates_input(branch: Option<&str>, refs: &str) -> String {
    let own = branch.map(|b| format!("refs/heads/{b}"));
    let mut input = String::from("HEAD\n");
    for r in refs
        .lines()
        .filter(|r| !r.is_empty() && Some(*r) != own.as_deref())
    {
        input.push('^');
        input.push_str(r);
        input.push('\n');
    }
    input
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    /// `None` — корневой коммит.
    pub parent: Option<String>,
    pub subject: String,
}

/// Разбирает [`candidates_args`]; останавливается на первом merge-коммите (его и всё ниже
/// не переписываем).
pub fn parse_candidates(out: &[u8]) -> Vec<Candidate> {
    let text = String::from_utf8_lossy(out);
    let fields: Vec<&str> = text.split('\0').map(|f| f.trim_start_matches('\n')).collect();
    let mut list = Vec::new();
    for rec in fields.chunks_exact(3) {
        let parents: Vec<&str> = rec[1].split_whitespace().collect();
        if parents.len() > 1 {
            break;
        }
        list.push(Candidate {
            id: rec[0].to_owned(),
            parent: parents.first().map(|p| (*p).to_owned()),
            subject: rec[2].to_owned(),
        });
    }
    list
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// Строка старой версии: первая заменяемая, а при `old_count == 0` — та, после которой
    /// вставка (0 — начало файла).
    pub old_start: usize,
    pub old_count: usize,
    /// Заголовок `@@` и строки фрагмента.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    pub path: String,
    /// Заголовок файла (`diff --git`, `---`, `+++`).
    pub header: String,
    pub hunks: Vec<Hunk>,
}

/// Разбирает [`staged_diff_args`]. Берёт только обычные изменения текстовых файлов:
/// новые, удалённые, бинарные файлы и смены режима остаются в индексе.
pub fn parse_diff(out: &[u8]) -> Vec<FilePatch> {
    let text = String::from_utf8_lossy(out);
    let mut files = Vec::new();
    for section in split_files(&text) {
        if let Some(f) = parse_file(section) {
            files.push(f);
        }
    }
    files
}

fn split_files(text: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = text
        .match_indices("diff --git ")
        .filter(|(i, _)| *i == 0 || text.as_bytes()[i - 1] == b'\n')
        .map(|(i, _)| i)
        .collect();
    starts.push(text.len());
    starts.windows(2).map(|w| &text[w[0]..w[1]]).collect()
}

fn parse_file(section: &str) -> Option<FilePatch> {
    let hunks_at = section.find("\n@@ ")? + 1;
    let header = &section[..hunks_at];
    let plain = header
        .lines()
        .skip(1)
        .all(|l| l.starts_with("index ") || l.starts_with("--- a/") || l.starts_with("+++ b/"));
    if !plain {
        return None;
    }
    let path = header.lines().find_map(|l| l.strip_prefix("+++ b/"))?.to_owned();
    let mut hunks: Vec<Hunk> = Vec::new();
    for line in section[hunks_at..].split_inclusive('\n') {
        if line.starts_with("@@ ") {
            let (old_start, old_count) = parse_range(line)?;
            hunks.push(Hunk {
                old_start,
                old_count,
                text: String::new(),
            });
        }
        hunks.last_mut()?.text.push_str(line);
    }
    Some(FilePatch {
        path,
        header: header.to_owned(),
        hunks,
    })
}

/// `@@ -12,3 +12,4 @@` → `(12, 3)`; без числа после запятой — одна строка.
fn parse_range(line: &str) -> Option<(usize, usize)> {
    let old = line.strip_prefix("@@ -")?.split(' ').next()?;
    let (start, count) = old.split_once(',').unwrap_or((old, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

/// Патч из выбранных фрагментов (`keep(файл, фрагмент)`); пусто, если ничего не выбрано.
pub fn build_patch(files: &[FilePatch], keep: impl Fn(usize, usize) -> bool) -> String {
    let mut out = String::new();
    for (fi, f) in files.iter().enumerate() {
        let hunks: Vec<&Hunk> = f
            .hunks
            .iter()
            .enumerate()
            .filter(|(hi, _)| keep(fi, *hi))
            .map(|(_, h)| h)
            .collect();
        if hunks.is_empty() {
            continue;
        }
        out.push_str(&f.header);
        for h in hunks {
            out.push_str(&h.text);
            if !h.text.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

pub fn blame_args(path: &str) -> Vec<String> {
    ["blame", "--porcelain", "HEAD", "--", path]
        .map(String::from)
        .to_vec()
}

/// Коммит каждой строки файла (индекс 0 — строка 1) из `blame --porcelain`.
pub fn parse_blame(out: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(out);
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines() {
        let mut parts = l.split(' ');
        let (Some(sha), Some(_orig), Some(fin)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let is_sha = sha.len() >= 40 && sha.bytes().all(|b| b.is_ascii_hexdigit());
        let Ok(fin) = fin.parse::<usize>() else { continue };
        if is_sha && fin >= 1 {
            if lines.len() < fin {
                lines.resize(fin, String::new());
            }
            lines[fin - 1] = sha.to_owned();
        }
    }
    lines
}

/// Коммит, которому принадлежит фрагмент: все заменяемые строки (или обе соседние строки
/// вставки) последними менял один и тот же коммит.
pub fn owner<'a>(hunk: &Hunk, blame: &'a [String]) -> Option<&'a str> {
    let lines: Vec<usize> = if hunk.old_count > 0 {
        (hunk.old_start..hunk.old_start + hunk.old_count).collect()
    } else {
        [hunk.old_start, hunk.old_start + 1]
            .into_iter()
            .filter(|n| (1..=blame.len()).contains(n))
            .collect()
    };
    let mut shas = lines.iter().map(|n| blame.get(n - 1).map(String::as_str));
    let first = shas.next()??;
    shas.all(|s| s == Some(first)).then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -2 +2 @@ x
-old
+new
@@ -5,0 +6,2 @@
+one
+two
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hi
";

    #[test]
    fn parses_plain_files_only() {
        let files = parse_diff(DIFF.as_bytes());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a.txt");
        let h = &files[0].hunks;
        assert_eq!((h[0].old_start, h[0].old_count), (2, 1));
        assert_eq!((h[1].old_start, h[1].old_count), (5, 0));
        assert_eq!(h[1].text, "@@ -5,0 +6,2 @@\n+one\n+two\n");
    }

    #[test]
    fn builds_patch_from_selected_hunks() {
        let files = parse_diff(DIFF.as_bytes());
        let p = build_patch(&files, |_, h| h == 1);
        assert!(p.starts_with("diff --git a/a.txt b/a.txt\n"));
        assert!(p.ends_with("@@ -5,0 +6,2 @@\n+one\n+two\n"));
        assert!(!p.contains("-old"));
        assert_eq!(build_patch(&files, |_, _| false), "");
    }

    #[test]
    fn owner_requires_single_commit() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let blame = vec![a.clone(), a.clone(), b.clone(), b.clone()];
        let h = |s, c| Hunk {
            old_start: s,
            old_count: c,
            text: String::new(),
        };
        assert_eq!(owner(&h(1, 2), &blame), Some(a.as_str()));
        assert_eq!(owner(&h(2, 2), &blame), None);
        // Вставка после строки 3: соседи 3 и 4 — оба b.
        assert_eq!(owner(&h(3, 0), &blame), Some(b.as_str()));
        // Между a и b — неоднозначно.
        assert_eq!(owner(&h(2, 0), &blame), None);
        // В конец файла и в начало — один сосед.
        assert_eq!(owner(&h(4, 0), &blame), Some(b.as_str()));
        assert_eq!(owner(&h(0, 0), &blame), Some(a.as_str()));
    }

    #[test]
    fn parses_blame_and_candidates() {
        let a = "a".repeat(40);
        let out = format!("{a} 1 1 2\nauthor X\n\tl1\n{a} 2 2\n\tl2\n");
        assert_eq!(parse_blame(out.as_bytes()), vec![a.clone(), a.clone()]);
        let log = "c3\0p2\0third\0\nc2\0p1\0second\0\nm\0x y\0merge\0\nc0\0\0root\0\n";
        let c = parse_candidates(log.as_bytes());
        assert_eq!(c.len(), 2);
        assert_eq!(
            c[1],
            Candidate {
                id: "c2".into(),
                parent: Some("p1".into()),
                subject: "second".into()
            }
        );
        let input = candidates_input(Some("f"), "refs/heads/f\nrefs/heads/main\nrefs/remotes/o/f\n");
        assert_eq!(input, "HEAD\n^refs/heads/main\n^refs/remotes/o/f\n");
        let root = parse_candidates(b"c0\0\0root\0\n");
        assert_eq!(root[0].parent, None);
    }
}
