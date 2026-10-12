//! Сборка patch из выбранных строк diff — для частичного stage/unstage
//! (`git apply --cached [--reverse]`).
//!
//! Stage берёт diff «рабочее дерево ↔ индекс» и применяется к индексу как есть.
//! Unstage берёт diff «HEAD ↔ индекс» и применяется к индексу в обратную сторону.
//! Невыбранные строки в обоих случаях превращаются в то, что уже лежит в индексе:
//! либо в контекст, либо выбрасываются.

use std::collections::BTreeSet;

use crate::diff::{FileDiff, LineKind};

/// Строка ханка: `(индекс ханка, индекс строки)`.
pub type LineRef = (usize, usize);

/// Почему из выбора нельзя собрать patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchError {
    NothingSelected,
    Binary,
    /// В файле не-UTF-8 текст, исходные байты не сохранились.
    NotUtf8,
    /// Переименование или удаление файла отменяется только целиком.
    WholeFileOnly,
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NothingSelected => "не выбрано ни одной изменённой строки",
            Self::Binary => "бинарный файл можно индексировать только целиком",
            Self::NotUtf8 => "файл не в UTF-8, его можно индексировать только целиком",
            Self::WholeFileOnly => "этот файл можно индексировать только целиком",
        })
    }
}

impl std::error::Error for PatchError {}

/// Все изменённые строки ханка — для кнопки «Stage ханк».
pub fn hunk_lines(file: &FileDiff, hunk: usize) -> BTreeSet<LineRef> {
    file.hunks[hunk]
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| matches!(l.kind, LineKind::Added | LineKind::Removed))
        .map(|(i, _)| (hunk, i))
        .collect()
}

/// Собирает patch для `git apply --cached` (`reverse = false`, stage) или
/// `git apply --cached --reverse` (`reverse = true`, unstage).
pub fn build(file: &FileDiff, selected: &BTreeSet<LineRef>, reverse: bool) -> Result<String, PatchError> {
    if file.binary {
        return Err(PatchError::Binary);
    }
    if file.lossy {
        return Err(PatchError::NotUtf8);
    }
    let changes: BTreeSet<LineRef> = (0..file.hunks.len()).flat_map(|h| hunk_lines(file, h)).collect();
    let chosen: BTreeSet<LineRef> = changes.intersection(selected).copied().collect();
    if chosen.is_empty() {
        return Err(PatchError::NothingSelected);
    }
    let whole = chosen.len() == changes.len();
    if file.is_rename() && !whole {
        return Err(PatchError::WholeFileOnly);
    }
    // Индекс без файла частично не восстановить: удаление снимается целиком.
    if reverse && file.new_path.is_none() && !whole {
        return Err(PatchError::WholeFileOnly);
    }

    let mut out = header(file, whole);
    let mut delta: i64 = 0;
    for (h, hunk) in file.hunks.iter().enumerate() {
        let mut body = String::new();
        let (mut old_count, mut new_count) = (0u32, 0u32);
        let mut changed = false;
        let mut prev_emitted = false;
        for (l, line) in hunk.lines.iter().enumerate() {
            let sel = chosen.contains(&(h, l));
            let marker = match (line.kind, sel, reverse) {
                (LineKind::Context, ..) => Some(' '),
                (LineKind::Added, true, _) => Some('+'),
                (LineKind::Removed, true, _) => Some('-'),
                // Невыбранная строка, которой в индексе нет, — выбрасывается,
                // а та, что в индексе есть, становится контекстом.
                (LineKind::Added, false, false) | (LineKind::Removed, false, true) => None,
                (LineKind::Added, false, true) | (LineKind::Removed, false, false) => Some(' '),
                (LineKind::NoNewline, ..) => {
                    if prev_emitted {
                        body.push_str("\\ No newline at end of file\n");
                    }
                    continue;
                }
            };
            prev_emitted = marker.is_some();
            let Some(m) = marker else { continue };
            match m {
                '+' => new_count += 1,
                '-' => old_count += 1,
                _ => {
                    old_count += 1;
                    new_count += 1;
                }
            }
            changed |= m != ' ';
            body.push(m);
            body.push_str(&line.text);
            if line.cr {
                body.push('\r');
            }
            body.push('\n');
        }
        if !changed {
            continue;
        }
        // Сторона, к которой patch применяется, совпадает с индексом и берётся как есть;
        // вторая сдвигается на изменения уже собранных ханков.
        let (old_start, new_start) = if reverse {
            (
                shift(hunk.new_start, delta, old_count),
                start(hunk.new_start, new_count),
            )
        } else {
            (
                start(hunk.old_start, old_count),
                shift(hunk.old_start, delta, new_count),
            )
        };
        delta += if reverse {
            i64::from(old_count) - i64::from(new_count)
        } else {
            i64::from(new_count) - i64::from(old_count)
        };
        out.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
        ));
        out.push_str(&body);
    }
    Ok(out)
}

/// У пустой стороны ханка git пишет номер строки перед ней (`-0,0`, `+4,0`).
fn start(line: u32, count: u32) -> u32 {
    if count == 0 {
        line.saturating_sub(1)
    } else {
        line.max(1)
    }
}

fn shift(line: u32, delta: i64, count: u32) -> u32 {
    let shifted = (i64::from(line.max(1)) + delta).max(1) as u32;
    if count == 0 {
        shifted.saturating_sub(1)
    } else {
        shifted
    }
}

fn header(file: &FileDiff, whole: bool) -> String {
    let path = file.path();
    let old = file.old_path.as_deref().unwrap_or(path);
    let new = file.new_path.as_deref().unwrap_or(path);
    let mode = file.mode.as_deref().unwrap_or("100644");
    let mut h = format!("diff --git {} {}\n", quote("a/", old), quote("b/", new));
    // Новый файл частично всё равно создаётся (с выбранными строками);
    // удалённый частично — это правка, файл остаётся.
    let added = file.old_path.is_none();
    let deleted = file.new_path.is_none() && whole;
    if added {
        h.push_str(&format!("new file mode {mode}\n"));
    }
    if deleted {
        h.push_str(&format!("deleted file mode {mode}\n"));
    }
    h.push_str(&format!(
        "--- {}\n+++ {}\n",
        if added {
            "/dev/null".to_owned()
        } else {
            quote("a/", old)
        },
        if deleted {
            "/dev/null".to_owned()
        } else {
            quote("b/", new)
        },
    ));
    h
}

/// Путь в заголовке patch; со спецсимволами — в C-кавычках, как пишет git.
fn quote(prefix: &str, path: &str) -> String {
    let needs = path.chars().any(|c| c == '"' || c == '\\' || c.is_control());
    if !needs {
        return format!("{prefix}{path}");
    }
    let mut q = String::from("\"");
    q.push_str(prefix);
    for c in path.chars() {
        match c {
            '"' => q.push_str("\\\""),
            '\\' => q.push_str("\\\\"),
            '\n' => q.push_str("\\n"),
            '\t' => q.push_str("\\t"),
            '\r' => q.push_str("\\r"),
            c if c.is_control() => q.push_str(&format!("\\{:03o}", c as u32)),
            c => q.push(c),
        }
    }
    q.push('"');
    q
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffLine, Hunk};

    fn line(kind: LineKind, text: &str) -> DiffLine {
        DiffLine {
            kind,
            old_no: None,
            new_no: None,
            text: text.into(),
            cr: false,
            spans: vec![],
        }
    }

    /// Файл `f.txt`: было `a b c d`, стало `a B c d e`.
    fn modified() -> FileDiff {
        use LineKind::*;
        FileDiff {
            old_path: Some("f.txt".into()),
            new_path: Some("f.txt".into()),
            hunks: vec![Hunk {
                header: String::new(),
                old_start: 1,
                old_count: 4,
                new_start: 1,
                new_count: 5,
                lines: vec![
                    line(Context, "a"),
                    line(Removed, "b"),
                    line(Added, "B"),
                    line(Context, "c"),
                    line(Context, "d"),
                    line(Added, "e"),
                ],
            }],
            ..Default::default()
        }
    }

    fn sel(refs: &[LineRef]) -> BTreeSet<LineRef> {
        refs.iter().copied().collect()
    }

    #[test]
    fn stage_only_added_line() {
        let p = build(&modified(), &sel(&[(0, 5)]), false).unwrap();
        assert_eq!(
            p,
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n\
             @@ -1,4 +1,5 @@\n a\n b\n c\n d\n+e\n"
        );
    }

    #[test]
    fn stage_only_removed_line_drops_unselected_addition() {
        let p = build(&modified(), &sel(&[(0, 1)]), false).unwrap();
        assert!(p.ends_with("@@ -1,4 +1,3 @@\n a\n-b\n c\n d\n"), "{p}");
    }

    #[test]
    fn unstage_keeps_unselected_additions_as_context() {
        // Обратное применение: в индексе уже `a B c d e`, снимаем только `e`.
        let p = build(&modified(), &sel(&[(0, 5)]), true).unwrap();
        assert!(p.ends_with("@@ -1,4 +1,5 @@\n a\n B\n c\n d\n+e\n"), "{p}");
    }

    #[test]
    fn whole_hunk_equals_original_and_skips_untouched_hunks() {
        let mut f = modified();
        let mut second = f.hunks[0].clone();
        second.old_start = 20;
        second.new_start = 21;
        f.hunks.push(second);
        let p = build(&f, &hunk_lines(&f, 1), false).unwrap();
        assert_eq!(p.matches("@@").count(), 2, "только один ханк: {p}");
        assert!(p.contains("@@ -20,4 +20,5 @@\n a\n-b\n+B\n c\n d\n+e\n"), "{p}");
        // Оба ханка: второй сдвинут на +1 строку первого.
        let all: BTreeSet<_> = hunk_lines(&f, 0).union(&hunk_lines(&f, 1)).copied().collect();
        let p = build(&f, &all, false).unwrap();
        assert!(p.contains("@@ -20,4 +21,5 @@"), "{p}");
    }

    #[test]
    fn new_file_partial_and_crlf() {
        let mut f = FileDiff {
            old_path: None,
            new_path: Some("n.txt".into()),
            mode: Some("100755".into()),
            hunks: vec![Hunk {
                header: String::new(),
                old_start: 0,
                old_count: 0,
                new_start: 1,
                new_count: 2,
                lines: vec![line(LineKind::Added, "x"), line(LineKind::Added, "y")],
            }],
            ..Default::default()
        };
        f.hunks[0].lines[1].cr = true;
        let p = build(&f, &sel(&[(0, 1)]), false).unwrap();
        assert_eq!(
            p,
            "diff --git a/n.txt b/n.txt\nnew file mode 100755\n--- /dev/null\n+++ b/n.txt\n\
             @@ -0,0 +1,1 @@\n+y\r\n"
        );
    }

    #[test]
    fn no_newline_marker_follows_emitted_line_only() {
        let mut f = modified();
        f.hunks[0]
            .lines
            .push(line(LineKind::NoNewline, "\\ No newline at end of file"));
        let with = build(&f, &sel(&[(0, 5)]), false).unwrap();
        assert!(with.ends_with("+e\n\\ No newline at end of file\n"), "{with}");
        let without = build(&f, &sel(&[(0, 1)]), false).unwrap();
        assert!(!without.contains("No newline"), "{without}");
    }

    #[test]
    fn deleted_file_whole_or_partial() {
        let f = FileDiff {
            old_path: Some("d.txt".into()),
            new_path: None,
            mode: Some("100644".into()),
            hunks: vec![Hunk {
                header: String::new(),
                old_start: 1,
                old_count: 2,
                new_start: 0,
                new_count: 0,
                lines: vec![line(LineKind::Removed, "1"), line(LineKind::Removed, "2")],
            }],
            ..Default::default()
        };
        let whole = build(&f, &hunk_lines(&f, 0), false).unwrap();
        assert!(whole.contains("deleted file mode 100644\n--- a/d.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@"));
        let part = build(&f, &sel(&[(0, 0)]), false).unwrap();
        assert!(
            part.contains("--- a/d.txt\n+++ b/d.txt\n@@ -1,2 +1,1 @@\n-1\n 2\n"),
            "{part}"
        );
        assert_eq!(build(&f, &sel(&[(0, 0)]), true), Err(PatchError::WholeFileOnly));
    }

    #[test]
    fn refusals() {
        let f = modified();
        assert_eq!(
            build(&f, &sel(&[(0, 0)]), false),
            Err(PatchError::NothingSelected)
        );
        let binary = FileDiff {
            binary: true,
            ..modified()
        };
        assert_eq!(build(&binary, &sel(&[(0, 1)]), false), Err(PatchError::Binary));
        let lossy = FileDiff {
            lossy: true,
            ..modified()
        };
        assert_eq!(build(&lossy, &sel(&[(0, 1)]), false), Err(PatchError::NotUtf8));
    }

    #[test]
    fn quotes_special_paths() {
        assert_eq!(quote("a/", "простой файл.txt"), "a/простой файл.txt");
        assert_eq!(quote("a/", "tab\there\"q"), "\"a/tab\\there\\\"q\"");
    }
}
