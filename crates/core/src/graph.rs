//! Раскладка графа коммитов по дорожкам (lanes). Считается в фоне вместе с историей,
//! UI только рисует готовые строки.

use std::collections::HashMap;

use crate::model::{Commit, Oid};

/// Отрезок графа внутри одной строки истории.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub lane: u16,
    /// Индекс цвета ветки; палитру выбирает UI.
    pub color: u16,
}

/// Что рисовать в строке одного коммита. Координаты — номера дорожек.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphRow {
    pub lane: u16,
    pub color: u16,
    /// Линии, проходящие строку насквозь сверху вниз по своей дорожке.
    pub through: Vec<Edge>,
    /// Линии от верхнего края дорожки к узлу коммита (дочерние коммиты).
    pub incoming: Vec<Edge>,
    /// Линии от узла коммита к нижнему краю дорожки (родители).
    pub outgoing: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitGraph {
    /// По строке на коммит, в порядке истории.
    pub rows: Vec<GraphRow>,
    /// Наибольшее число дорожек в одной строке — ширина колонки графа.
    pub width: u16,
}

#[derive(Clone)]
struct Lane {
    /// Коммит, к которому идёт линия дорожки.
    wants: Oid,
    color: u16,
}

impl CommitGraph {
    /// Раскладывает историю, упорядоченную от новых к старым (дети раньше родителей,
    /// как у `git log --date-order`). Дорожки не сдвигаются: освободившаяся дорожка
    /// занимается следующей новой веткой, поэтому сквозные линии всегда вертикальны.
    pub fn build(commits: &[Commit]) -> Self {
        let mut lanes: Vec<Option<Lane>> = vec![];
        let mut next_color: u16 = 0;
        let mut rows = Vec::with_capacity(commits.len());
        let mut width = 0usize;
        let mut new_color = || {
            let c = next_color;
            next_color = next_color.wrapping_add(1);
            c
        };

        for commit in commits {
            let mut row = GraphRow::default();
            let before = lanes.clone();

            // Дорожки, которые ждут этот коммит, сходятся в его узел.
            let waiting: Vec<usize> = (0..lanes.len())
                .filter(|&i| lanes[i].as_ref().is_some_and(|l| l.wants == commit.id))
                .collect();
            let (node, color) = match waiting.first() {
                Some(&i) => (i, lanes[i].as_ref().map_or(0, |l| l.color)),
                None => (free_lane(&mut lanes), new_color()),
            };
            for &i in &waiting {
                let l = lanes[i].take().expect("ожидающая дорожка");
                row.incoming.push(edge(i, l.color));
            }

            // Родители: первый всегда продолжает дорожку узла, даже если его уже ждёт
            // другая дорожка — тогда обе сойдутся в родителе, а ветка не прыгает вбок.
            // Второй и следующие (слияния) уходят в дорожку, которая уже ждёт родителя,
            // или получают новую.
            let mut targets: HashMap<usize, u16> = HashMap::new();
            for (k, parent) in commit.parents.iter().enumerate() {
                let existing = if k == 0 && lanes[node].is_none() {
                    None
                } else {
                    lanes
                        .iter()
                        .position(|l| l.as_ref().is_some_and(|l| &l.wants == parent))
                };
                let (lane, c) = match existing {
                    Some(i) => (i, lanes[i].as_ref().map_or(0, |l| l.color)),
                    None => {
                        let (i, c) = if k == 0 && lanes[node].is_none() {
                            (node, color)
                        } else {
                            (free_lane(&mut lanes), new_color())
                        };
                        lanes[i] = Some(Lane {
                            wants: parent.clone(),
                            color: c,
                        });
                        (i, c)
                    }
                };
                targets.entry(lane).or_insert(c);
            }
            let mut out: Vec<_> = targets.into_iter().collect();
            out.sort_unstable();
            row.outgoing = out.into_iter().map(|(i, c)| edge(i, c)).collect();

            for (i, l) in before.iter().enumerate() {
                if let Some(l) = l {
                    if l.wants != commit.id {
                        row.through.push(edge(i, l.color));
                    }
                }
            }
            while lanes.last().is_some_and(Option::is_none) {
                lanes.pop();
            }

            width = width.max(before.len()).max(lanes.len()).max(node + 1);
            row.lane = node as u16;
            row.color = color;
            rows.push(row);
        }

        Self {
            rows,
            width: width as u16,
        }
    }
}

fn edge(lane: usize, color: u16) -> Edge {
    Edge {
        lane: lane as u16,
        color,
    }
}

/// Первая свободная дорожка; если свободных нет — новая справа.
fn free_lane(lanes: &mut Vec<Option<Lane>>) -> usize {
    match lanes.iter().position(Option::is_none) {
        Some(i) => i,
        None => {
            lanes.push(None);
            lanes.len() - 1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, parents: &[&str]) -> Commit {
        Commit {
            id: id.into(),
            parents: parents.iter().map(|p| (*p).into()).collect(),
            ..Default::default()
        }
    }

    fn lanes(e: &[Edge]) -> Vec<u16> {
        e.iter().map(|e| e.lane).collect()
    }

    #[test]
    fn linear_history_is_one_lane() {
        let g = CommitGraph::build(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert_eq!(g.width, 1);
        assert!(g.rows.iter().all(|r| r.lane == 0 && r.through.is_empty()));
        assert!(g.rows[0].incoming.is_empty());
        assert_eq!(lanes(&g.rows[1].incoming), [0]);
        assert_eq!(lanes(&g.rows[1].outgoing), [0]);
        assert!(g.rows[2].outgoing.is_empty());
    }

    #[test]
    fn branch_and_merge() {
        // m — слияние f в main; f и b ответвились от a.
        let g = CommitGraph::build(&[
            commit("m", &["b", "f"]),
            commit("f", &["a"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ]);
        assert_eq!(g.width, 2);
        let [m, f, b, a] = &g.rows[..] else { panic!() };
        assert_eq!((m.lane, lanes(&m.outgoing)), (0, vec![0, 1]));
        assert_eq!((f.lane, lanes(&f.through)), (1, vec![0]));
        assert_eq!(lanes(&f.outgoing), [1]);
        // b продолжает main; линия f к a идёт насквозь по дорожке 1.
        assert_eq!((b.lane, lanes(&b.through)), (0, vec![1]));
        // Обе дорожки сходятся в общего предка.
        assert_eq!((a.lane, lanes(&a.incoming)), (0, vec![0, 1]));
        assert_ne!(m.outgoing[0].color, m.outgoing[1].color);
        assert_eq!(f.color, m.outgoing[1].color);
    }

    #[test]
    fn separate_branch_tips_get_own_lanes_and_free_lanes_are_reused() {
        let g = CommitGraph::build(&[
            commit("x", &["a"]),
            commit("y", &["a"]),
            commit("a", &[]),
            commit("z", &[]),
        ]);
        assert_eq!(g.rows[0].lane, 0);
        assert_eq!(g.rows[1].lane, 1);
        assert_eq!(lanes(&g.rows[1].outgoing), [1]);
        assert_eq!((g.rows[2].lane, lanes(&g.rows[2].incoming)), (0, vec![0, 1]));
        // После корня a все дорожки свободны, новый корень z встаёт в дорожку 0.
        assert_eq!(g.rows[3].lane, 0);
        assert!(g.rows[3].through.is_empty());
    }

    #[test]
    fn parent_outside_loaded_history_keeps_line_to_bottom() {
        let g = CommitGraph::build(&[commit("b", &["a"])]);
        assert_eq!(lanes(&g.rows[0].outgoing), [0]);
    }

    #[test]
    fn empty_history() {
        assert_eq!(CommitGraph::build(&[]), CommitGraph::default());
    }
}
