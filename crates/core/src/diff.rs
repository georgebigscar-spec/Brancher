//! Модель diff: файлы, ханки, строки. Заполняется парсером unified diff из `brancher-git`.

use crate::model::Oid;

/// Что сравнивать.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiffTarget {
    /// Изменения коммита относительно первого родителя (`None` — корневой коммит).
    Commit { id: Oid, parent: Option<Oid> },
    /// Файл рабочей копии: индекс против HEAD (`staged`) или рабочее дерево против индекса.
    Change {
        path: String,
        staged: bool,
        untracked: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file`.
    NoNewline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// Текст строки без маркера и без перевода строки.
    pub text: String,
    /// Строка заканчивалась на `\r\n` (нужно, чтобы собрать patch байт в байт).
    pub cr: bool,
    /// Подсветка синтаксиса; пусто — без подсветки. Заполняется в фоне после разбора.
    pub spans: Vec<Span>,
}

/// Участок строки одного цвета: байты `start..end` в [`DiffLine::text`].
/// Цвета для светлой и тёмной темы считаются сразу, чтобы смена темы не пересчитывала diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub light: [u8; 3],
    pub dark: [u8; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// Строка `@@ -a,b +c,d @@ …` целиком.
    pub header: String,
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileDiff {
    /// Путь до изменения; `None` — файл добавлен.
    pub old_path: Option<String>,
    /// Путь после изменения; `None` — файл удалён.
    pub new_path: Option<String>,
    pub binary: bool,
    /// Режим из `new file mode` / `deleted file mode`.
    pub mode: Option<String>,
    /// В тексте были не-UTF-8 байты: показать можно, собрать patch — нет.
    pub lossy: bool,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    /// Путь для показа: новый, а для удалённого файла — старый.
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default()
    }

    pub fn is_rename(&self) -> bool {
        matches!((&self.old_path, &self.new_path), (Some(a), Some(b)) if a != b)
    }

    /// Число добавленных и удалённых строк.
    pub fn stats(&self) -> (usize, usize) {
        let lines = self.hunks.iter().flat_map(|h| &h.lines);
        lines.fold((0, 0), |(a, r), l| match l.kind {
            LineKind::Added => (a + 1, r),
            LineKind::Removed => (a, r + 1),
            _ => (a, r),
        })
    }
}

/// Ширина табуляции при показе diff.
pub const TAB_WIDTH: usize = 4;

/// Ширина строки в символах с учётом табуляций.
pub fn display_width(text: &str) -> usize {
    text.chars().map(|c| if c == '\t' { TAB_WIDTH } else { 1 }).sum()
}

/// Строка панели diff: заголовок файла, заголовок ханка или строка кода.
/// Плоский список нужен виртуальной прокрутке (рисуются только видимые строки).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRow {
    File(usize),
    Hunk(usize, usize),
    Line(usize, usize, usize),
}

/// Строка side-by-side режима: слева старая версия, справа новая.
/// Удалённые и добавленные строки одного блока стоят друг напротив друга.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitRow {
    File(usize),
    Hunk(usize, usize),
    /// `(файл, ханк, строка слева, строка справа)`; у контекста обе стороны — одна строка.
    Pair(usize, usize, Option<usize>, Option<usize>),
}

/// Загруженный diff вместе с плоским списком строк для показа.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diff {
    pub files: Vec<FileDiff>,
    pub rows: Vec<DiffRow>,
    /// Те же строки для side-by-side режима.
    pub split_rows: Vec<SplitRow>,
    /// Самая длинная строка кода в символах — ширина для горизонтальной прокрутки.
    pub max_line_len: usize,
}

impl Diff {
    pub fn new(files: Vec<FileDiff>) -> Self {
        let mut rows = vec![];
        for (f, file) in files.iter().enumerate() {
            rows.push(DiffRow::File(f));
            for (h, hunk) in file.hunks.iter().enumerate() {
                rows.push(DiffRow::Hunk(f, h));
                rows.extend((0..hunk.lines.len()).map(|l| DiffRow::Line(f, h, l)));
            }
        }
        let max_line_len = files
            .iter()
            .flat_map(|f| &f.hunks)
            .flat_map(|h| &h.lines)
            .map(|l| display_width(&l.text))
            .max()
            .unwrap_or(0);
        let split_rows = split_rows(&files);
        Self {
            files,
            rows,
            split_rows,
            max_line_len,
        }
    }
}

fn split_rows(files: &[FileDiff]) -> Vec<SplitRow> {
    let mut rows = vec![];
    for (f, file) in files.iter().enumerate() {
        rows.push(SplitRow::File(f));
        for (h, hunk) in file.hunks.iter().enumerate() {
            rows.push(SplitRow::Hunk(f, h));
            // Подряд идущие удалённые и добавленные строки — один блок замены.
            let (mut removed, mut added) = (vec![], vec![]);
            let flush = |rows: &mut Vec<SplitRow>, removed: &mut Vec<usize>, added: &mut Vec<usize>| {
                for i in 0..removed.len().max(added.len()) {
                    rows.push(SplitRow::Pair(
                        f,
                        h,
                        removed.get(i).copied(),
                        added.get(i).copied(),
                    ));
                }
                removed.clear();
                added.clear();
            };
            let mut last = LineKind::Context;
            for (l, line) in hunk.lines.iter().enumerate() {
                match line.kind {
                    LineKind::Removed => {
                        if !added.is_empty() {
                            flush(&mut rows, &mut removed, &mut added);
                        }
                        removed.push(l);
                    }
                    LineKind::Added => added.push(l),
                    // Маркер «нет перевода строки» относится к стороне предыдущей строки.
                    LineKind::NoNewline => match last {
                        LineKind::Removed => removed.push(l),
                        LineKind::Added => added.push(l),
                        _ => rows.push(SplitRow::Pair(f, h, Some(l), Some(l))),
                    },
                    LineKind::Context => {
                        flush(&mut rows, &mut removed, &mut added);
                        rows.push(SplitRow::Pair(f, h, Some(l), Some(l)));
                    }
                }
                if line.kind != LineKind::NoNewline {
                    last = line.kind;
                }
            }
            flush(&mut rows, &mut removed, &mut added);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind) -> DiffLine {
        DiffLine {
            kind,
            old_no: None,
            new_no: None,
            text: String::new(),
            cr: false,
            spans: vec![],
        }
    }

    #[test]
    fn rows_flatten_files_hunks_and_lines() {
        let hunk = |n| Hunk {
            header: "@@".into(),
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
            lines: vec![line(LineKind::Context); n],
        };
        let d = Diff::new(vec![
            FileDiff {
                hunks: vec![hunk(2), hunk(1)],
                ..Default::default()
            },
            FileDiff {
                binary: true,
                ..Default::default()
            },
        ]);
        use DiffRow as R;
        assert_eq!(
            d.rows,
            [
                R::File(0),
                R::Hunk(0, 0),
                R::Line(0, 0, 0),
                R::Line(0, 0, 1),
                R::Hunk(0, 1),
                R::Line(0, 1, 0),
                R::File(1)
            ]
        );
    }

    #[test]
    fn split_rows_pair_removed_with_added() {
        use LineKind::*;
        let kinds = [Context, Removed, Removed, Added, Context, Added, NoNewline];
        let d = Diff::new(vec![FileDiff {
            hunks: vec![Hunk {
                header: "@@".into(),
                old_start: 1,
                old_count: 4,
                new_start: 1,
                new_count: 4,
                lines: kinds.iter().map(|k| line(*k)).collect(),
            }],
            ..Default::default()
        }]);
        use SplitRow as S;
        assert_eq!(
            d.split_rows,
            [
                S::File(0),
                S::Hunk(0, 0),
                S::Pair(0, 0, Some(0), Some(0)),
                S::Pair(0, 0, Some(1), Some(3)),
                S::Pair(0, 0, Some(2), None),
                S::Pair(0, 0, Some(4), Some(4)),
                S::Pair(0, 0, None, Some(5)),
                S::Pair(0, 0, None, Some(6)),
            ]
        );
    }

    #[test]
    fn stats_and_paths() {
        let f = FileDiff {
            old_path: Some("a.txt".into()),
            new_path: None,
            hunks: vec![Hunk {
                header: String::new(),
                old_start: 1,
                old_count: 2,
                new_start: 0,
                new_count: 0,
                lines: vec![line(LineKind::Removed), line(LineKind::Removed)],
            }],
            ..Default::default()
        };
        assert_eq!(f.path(), "a.txt");
        assert_eq!(f.stats(), (0, 2));
        assert!(!f.is_rename());
    }
}
