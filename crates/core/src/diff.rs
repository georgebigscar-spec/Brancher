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

/// Загруженный diff вместе с плоским списком строк для показа.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diff {
    pub files: Vec<FileDiff>,
    pub rows: Vec<DiffRow>,
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
        Self {
            files,
            rows,
            max_line_len,
        }
    }
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
