//! Lane assignment for drawing the commit graph next to the history list.
//!
//! Each row is split into a top half (from the row above down to the commit dot) and a bottom half
//! (from the dot down to the row below). Lines are expressed per half, so the UI can draw every row
//! independently, which keeps virtualized scrolling cheap.

use std::collections::HashSet;

use crate::{CommitInfo, Oid};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Half {
    Top,
    Bottom,
}

/// A line segment within one half of a row, between two lane columns. Kept small (12 bytes): a long history
/// holds millions of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphLine {
    pub half: Half,
    /// Column at the upper end of the segment.
    pub from: u16,
    /// Column at the lower end of the segment.
    pub to: u16,
    pub color: u32,
}

impl GraphLine {
    fn new(half: Half, from: usize, to: usize, color: usize) -> Self {
        // Columns are folded to MAX_LANES afterwards; colors only pick from a palette, so wrapping is harmless.
        let column = |c: usize| u16::try_from(c).unwrap_or(u16::MAX);
        Self { half, from: column(from), to: column(to), color: color as u32 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    /// Column of the commit dot.
    pub column: usize,
    pub color: usize,
    pub lines: Vec<GraphLine>,
    /// Number of columns used by this row.
    pub width: usize,
}

/// Lanes past this many columns share the last one: they would be too narrow to tell apart, and keeping each
/// costs a lot of memory on histories with hundreds of parallel lanes (git.git averages 168 per row).
pub const MAX_LANES: usize = 24;

#[derive(Clone, Copy)]
struct Lane {
    expects: Oid,
    color: usize,
}

/// Computes one [`GraphRow`] per commit. `commits` must list children before their parents.
pub fn layout(commits: &[CommitInfo]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut next_color = 0;
    let mut new_color = || {
        next_color += 1;
        next_color - 1
    };
    // Parents that are not part of the list must not keep a lane open forever.
    let listed: HashSet<Oid> = commits.iter().map(|c| c.id).collect();

    commits
        .iter()
        .map(|commit| {
            let incoming: Vec<usize> =
                lanes.iter().enumerate().filter(|(_, l)| l.is_some_and(|l| l.expects == commit.id)).map(|(i, _)| i).collect();

            let column = incoming.first().copied().unwrap_or_else(|| free_slot(&mut lanes));
            let color = lanes[column].map_or_else(&mut new_color, |l| l.color);
            let mut row = RowBuilder::new(column, color);

            // Top half: lanes passing through, and lanes converging into this commit.
            for (i, lane) in lanes.iter().enumerate() {
                let Some(lane) = lane else { continue };
                let to = if lane.expects == commit.id { column } else { i };
                row.line(Half::Top, i, to, lane.color);
            }
            incoming.iter().for_each(|&i| lanes[i] = None);

            // Bottom half: one line per parent, then lanes passing through.
            let parents: Vec<Oid> = commit.parents.iter().copied().filter(|p| listed.contains(p)).collect();
            let mut fresh = Vec::new();
            for (n, parent) in parents.iter().enumerate() {
                let existing = lanes.iter().position(|l| l.is_some_and(|l| l.expects == *parent));
                let (target, lane_color) = match existing {
                    Some(j) => (j, lanes[j].expect("position found it").color),
                    None if n == 0 => {
                        lanes[column] = Some(Lane { expects: *parent, color });
                        fresh.push(column);
                        (column, color)
                    }
                    None => {
                        let slot = free_slot(&mut lanes);
                        let lane_color = new_color();
                        lanes[slot] = Some(Lane { expects: *parent, color: lane_color });
                        fresh.push(slot);
                        (slot, lane_color)
                    }
                };
                row.line(Half::Bottom, column, target, lane_color);
            }
            for (i, lane) in lanes.iter().enumerate() {
                if let Some(lane) = lane
                    && !fresh.contains(&i)
                {
                    row.line(Half::Bottom, i, i, lane.color);
                }
            }

            while lanes.last().is_some_and(Option::is_none) {
                lanes.pop();
            }
            row.finish()
        })
        .collect()
}

/// Collects one row's lines, moving everything past [`MAX_LANES`] into the last column as it goes. Folding while
/// adding, rather than adding every line and deduplicating afterwards, matters on histories with hundreds of
/// parallel lanes: nearly all of them collapse into the same few lines.
struct RowBuilder {
    row: GraphRow,
    /// Folded lines already added, per half: by their lower column when the upper one was folded, else by their
    /// upper column (offset by `MAX_LANES`) when only the lower one was.
    folded: [[bool; 2 * MAX_LANES]; 2],
}

impl RowBuilder {
    fn new(column: usize, color: usize) -> Self {
        let row = GraphRow { column, color, lines: Vec::new(), width: column + 1 };
        Self { row, folded: [[false; 2 * MAX_LANES]; 2] }
    }

    fn line(&mut self, half: Half, from: usize, to: usize, color: usize) {
        self.row.width = self.row.width.max(from.max(to) + 1);
        let last = MAX_LANES - 1;
        let (folded_from, folded_to) = (from.min(last), to.min(last));
        if from >= last || to >= last {
            let slot = if folded_from == last { folded_to } else { MAX_LANES + folded_from };
            let seen = &mut self.folded[half as usize][slot];
            if std::mem::replace(seen, true) {
                return;
            }
        }
        self.row.lines.push(GraphLine::new(half, folded_from, folded_to, color));
    }

    fn finish(mut self) -> GraphRow {
        if self.row.width > MAX_LANES {
            self.row.width = MAX_LANES;
            self.row.column = self.row.column.min(MAX_LANES - 1);
        }
        // Rows are kept for the whole history, so give back unused room.
        self.row.lines.shrink_to_fit();
        self.row
    }
}

fn free_slot(lanes: &mut Vec<Option<Lane>>) -> usize {
    lanes.iter().position(Option::is_none).unwrap_or_else(|| {
        lanes.push(None);
        lanes.len() - 1
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::commit;

    #[test]
    fn linear_history_stays_in_one_column() {
        let rows = layout(&[commit(3, &[2]), commit(2, &[1]), commit(1, &[])]);
        assert!(rows.iter().all(|r| r.column == 0 && r.width == 1));
    }

    #[test]
    fn lanes_past_the_limit_share_the_last_column() {
        // 30 branch tips listed before any of their 30 bases: 30 lanes run in parallel.
        let tips = (1..=30).map(|n| commit(n, &[n + 100]));
        let bases = (101..=130).map(|n| commit(n, &[]));
        let rows = layout(&tips.chain(bases).collect::<Vec<_>>());
        assert!(rows.iter().any(|r| r.width == MAX_LANES), "the limit is reached");
        for row in &rows {
            assert!(row.width <= MAX_LANES && row.column < MAX_LANES);
            assert!(row.lines.iter().all(|l| usize::from(l.from.max(l.to)) < MAX_LANES));
            let unique: HashSet<_> = row.lines.iter().map(|l| (l.half, l.from, l.to)).collect();
            assert_eq!(unique.len(), row.lines.len(), "no line is stored twice");
        }
    }

    #[test]
    fn merge_opens_and_closes_a_lane() {
        // 4 merges 3 (first parent) and 2; both descend from 1.
        let rows = layout(&[commit(4, &[3, 2]), commit(3, &[1]), commit(2, &[1]), commit(1, &[])]);
        assert_eq!(rows[0].column, 0);
        assert_eq!(rows[1].column, 0);
        assert_eq!(rows[2].column, 1);
        assert_eq!(rows[3].column, 0);
        assert!(rows[2].lines.iter().any(|l| l.half == Half::Bottom && l.from == 1 && l.to == 0));
        assert_eq!(rows[3].width, 1);
    }
}
