//! Подсветка синтаксиса строк diff (syntect). Выполняется в worker-потоке сразу после
//! разбора diff, UI получает готовые цвета.
//!
//! Старая и новая стороны каждого ханка подсвечиваются отдельно и с начала ханка:
//! состояния парсера до ханка у нас нет, поэтому многострочные конструкции
//! (комментарий, начатый выше ханка) могут подсветиться неточно — как и в других клиентах.

use std::sync::OnceLock;

use brancher_core::{FileDiff, Hunk, LineKind, Span};
use syntect::highlighting::{Color, HighlightIterator, HighlightState, Highlighter, Theme, ThemeSet};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

/// Не подсвечиваем огромные diff и очень длинные строки: это минифицированный
/// или сгенерированный код, а время worker-а нужнее другим задачам.
const MAX_LINES: usize = 20_000;
const MAX_LINE_LEN: usize = 2_000;

struct Assets {
    syntaxes: SyntaxSet,
    light: Theme,
    dark: Theme,
}

fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let mut themes = ThemeSet::load_defaults().themes;
        Assets {
            syntaxes: SyntaxSet::load_defaults_newlines(),
            light: themes.remove("InspiredGitHub").expect("тема из syntect"),
            dark: themes.remove("base16-ocean.dark").expect("тема из syntect"),
        }
    })
}

/// Заполняет [`brancher_core::DiffLine::spans`] у всех файлов, язык которых удалось определить.
pub fn highlight(files: &mut [FileDiff]) {
    let total: usize = files.iter().flat_map(|f| &f.hunks).map(|h| h.lines.len()).sum();
    if total > MAX_LINES {
        return;
    }
    let a = assets();
    for file in files.iter_mut().filter(|f| !f.binary) {
        let Some(syntax) = syntax_for(&a.syntaxes, file.path()) else {
            continue;
        };
        for hunk in &mut file.hunks {
            highlight_side(a, syntax, hunk, LineKind::Removed);
            highlight_side(a, syntax, hunk, LineKind::Added);
        }
    }
}

fn syntax_for<'a>(set: &'a SyntaxSet, path: &str) -> Option<&'a SyntaxReference> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or(name);
    set.find_syntax_by_extension(ext)
        .or_else(|| set.find_syntax_by_extension(name))
        .filter(|s| s.name != "Plain Text")
}

/// Одна сторона ханка: контекст плюс удалённые (`Removed`) или добавленные (`Added`) строки.
fn highlight_side(a: &Assets, syntax: &SyntaxReference, hunk: &mut Hunk, side: LineKind) {
    let mut parse = ParseState::new(syntax);
    let (hl_light, hl_dark) = (Highlighter::new(&a.light), Highlighter::new(&a.dark));
    let mut state_light = HighlightState::new(&hl_light, ScopeStack::new());
    let mut state_dark = HighlightState::new(&hl_dark, ScopeStack::new());

    for line in hunk.lines.iter_mut() {
        if line.kind != LineKind::Context && line.kind != side {
            continue;
        }
        if line.text.len() > MAX_LINE_LEN {
            // Состояние парсера после пропущенной строки неизвестно — дальше не подсвечиваем.
            return;
        }
        let text = format!("{}\n", line.text);
        let Ok(ops) = parse.parse_line(&text, &a.syntaxes) else {
            return;
        };
        let light = HighlightIterator::new(&mut state_light, &ops, &text, &hl_light).collect::<Vec<_>>();
        let dark = HighlightIterator::new(&mut state_dark, &ops, &text, &hl_dark).collect::<Vec<_>>();
        // Контекст есть на обеих сторонах: берём подсветку только с одной, чтобы не перезаписывать.
        if line.kind == LineKind::Context && side == LineKind::Added {
            continue;
        }
        line.spans = merge(&light, &dark, line.text.len());
    }
}

/// Сводит разбиения двух тем в одно: границы участков у тем могут не совпадать.
fn merge(
    light: &[(syntect::highlighting::Style, &str)],
    dark: &[(syntect::highlighting::Style, &str)],
    len: usize,
) -> Vec<Span> {
    let bounds = |runs: &[(syntect::highlighting::Style, &str)]| {
        let mut pos = 0;
        runs.iter()
            .map(|(style, s)| {
                pos += s.len();
                (pos.min(len), style.foreground)
            })
            .collect::<Vec<_>>()
    };
    let (l, d) = (bounds(light), bounds(dark));
    let mut cuts: Vec<usize> = l.iter().chain(&d).map(|(p, _)| *p).filter(|p| *p > 0).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let color_at = |runs: &[(usize, Color)], pos: usize| {
        let c = runs
            .iter()
            .find(|(end, _)| *end > pos)
            .map(|(_, c)| *c)
            .unwrap_or(Color::BLACK);
        [c.r, c.g, c.b]
    };
    let mut spans: Vec<Span> = vec![];
    let mut start = 0;
    for end in cuts {
        if end <= start {
            continue;
        }
        let (light, dark) = (color_at(&l, start), color_at(&d, start));
        match spans.last_mut() {
            Some(last) if last.light == light && last.dark == dark => last.end = end as u32,
            _ => spans.push(Span {
                start: start as u32,
                end: end as u32,
                light,
                dark,
            }),
        }
        start = end;
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use brancher_core::DiffLine;

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

    fn file(path: &str, lines: Vec<DiffLine>) -> FileDiff {
        FileDiff {
            old_path: Some(path.into()),
            new_path: Some(path.into()),
            hunks: vec![Hunk {
                header: String::new(),
                old_start: 1,
                old_count: 1,
                new_start: 1,
                new_count: 1,
                lines,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn rust_lines_get_colored_spans_covering_the_text() {
        let mut files = vec![file(
            "src/main.rs",
            vec![
                line(LineKind::Context, "fn main() {"),
                line(LineKind::Removed, "    let x = 1;"),
                line(LineKind::Added, "    let x = \"два\";"),
            ],
        )];
        highlight(&mut files);
        for l in &files[0].hunks[0].lines {
            assert!(l.spans.len() > 1, "{:?} без подсветки", l.text);
            assert_eq!(l.spans.first().unwrap().start, 0);
            assert_eq!(l.spans.last().unwrap().end as usize, l.text.len());
            assert!(l.spans.windows(2).all(|w| w[0].end == w[1].start));
            // Границы участков — на границах символов UTF-8.
            assert!(l.spans.iter().all(|s| l.text.is_char_boundary(s.end as usize)));
        }
        let kw = &files[0].hunks[0].lines[0].spans[0];
        assert_ne!(kw.light, kw.dark, "у тем разные цвета");
    }

    #[test]
    fn unknown_language_and_binary_stay_plain() {
        let mut files = vec![file(
            "notes.unknownext",
            vec![line(LineKind::Added, "просто текст")],
        )];
        files.push(FileDiff {
            binary: true,
            ..file("img.rs", vec![line(LineKind::Added, "fn x() {}")])
        });
        highlight(&mut files);
        assert!(files
            .iter()
            .flat_map(|f| &f.hunks)
            .flat_map(|h| &h.lines)
            .all(|l| l.spans.is_empty()));
    }
}
