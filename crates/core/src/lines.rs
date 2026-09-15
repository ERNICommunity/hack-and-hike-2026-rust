//! A fixed-size history of text lines, newest last.
//!
//! The firmware keeps the last lines of its log here, so the demo can show
//! them on screen. Nothing is allocated on the heap: the history is one
//! fixed array of fixed-capacity strings, used as a ring buffer.

use arrayvec::ArrayString;

/// Number of lines the history keeps. When it is full, a new line replaces
/// the oldest one.
pub const LINES: usize = 64;
/// Longest line the history keeps, in bytes. A longer line is cut and ends
/// in `...`.
pub const LINE_BYTES: usize = 120;
/// Appended to a line that was cut.
const CUT_MARK: &str = "...";

/// One line of history: a string of at most [`LINE_BYTES`] bytes.
pub type Line = ArrayString<LINE_BYTES>;

/// The last [`LINES`] lines pushed, oldest first.
pub struct LineHistory {
    /// Storage of the ring. When the ring is full, the slot at `next` holds
    /// the oldest line.
    lines: [Line; LINES],
    /// Index of the slot for the next line.
    next: usize,
    /// Lines kept: the lines pushed so far, but at most `LINES`.
    count: usize,
    /// Increases by one with every push and wraps around at `u32::MAX`.
    /// Readers compare it to skip a history that did not change.
    revision: u32,
}

impl Default for LineHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl LineHistory {
    /// An empty history.
    pub const fn new() -> Self {
        Self {
            lines: [Line::new_const(); LINES],
            next: 0,
            count: 0,
            revision: 0,
        }
    }

    /// Increments with every [`push`](Self::push); compare it to see
    /// whether anything new arrived.
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// Lines currently kept, at most [`LINES`].
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Whether no line was pushed yet.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Remember `text`, without the `\r` and `\n` characters at its end.
    /// A text longer than [`LINE_BYTES`] is cut and ends in `...`.
    pub fn push(&mut self, text: &str) {
        let text = text.trim_end_matches(['\r', '\n']);
        let line = &mut self.lines[self.next];
        line.clear();
        if line.try_push_str(text).is_err() {
            let keep = floor_char_boundary(text, LINE_BYTES - CUT_MARK.len());
            line.push_str(&text[..keep]);
            line.push_str(CUT_MARK);
        }
        self.next = (self.next + 1) % LINES;
        self.count = (self.count + 1).min(LINES);
        self.revision = self.revision.wrapping_add(1);
    }

    /// The lines in order, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Line> {
        let start = (self.next + LINES - self.count) % LINES;
        (0..self.count).map(move |offset| &self.lines[(start + offset) % LINES])
    }

    /// Copy the newest lines into `out`, oldest first, as many as fit.
    /// Returns how many were copied.
    pub fn newest(&self, out: &mut [Line]) -> usize {
        let count = self.count.min(out.len());
        for (slot, line) in out.iter_mut().zip(self.iter().skip(self.count - count)) {
            *slot = *line;
        }
        count
    }
}

/// The largest byte index, at most `index`, that lies on a UTF-8 character
/// boundary of `text`. An `index` past the end is first reduced to
/// `text.len()`.
fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::{format, string::ToString as _};

    use super::*;

    #[test]
    fn keeps_the_newest_lines_in_order() {
        let mut history = LineHistory::new();
        assert!(history.is_empty());
        for n in 0..LINES + 3 {
            history.push(&format!("line {n}\n"));
        }
        assert_eq!(history.len(), LINES);
        assert_eq!(history.revision(), (LINES + 3) as u32);

        let first = history.iter().next().unwrap();
        assert_eq!(first.as_str(), "line 3");
        let last = history.iter().last().unwrap();
        assert_eq!(last.as_str(), format!("line {}", LINES + 2));
    }

    #[test]
    fn newest_copies_the_tail() {
        let mut history = LineHistory::new();
        for n in 0..10 {
            history.push(&format!("{n}"));
        }
        let mut out = [Line::new(); 3];
        assert_eq!(history.newest(&mut out), 3);
        assert_eq!(out.map(|line| line.to_string()), ["7", "8", "9"]);

        let mut plenty = [Line::new(); 20];
        assert_eq!(history.newest(&mut plenty), 10);
        assert_eq!(plenty[0].as_str(), "0");
    }

    #[test]
    fn long_lines_are_cut_at_a_character_boundary() {
        let mut history = LineHistory::new();
        let long = "ä".repeat(LINE_BYTES);
        history.push(&long);
        let line = history.iter().next().unwrap();
        assert!(line.ends_with("..."));
        assert!(line.len() <= LINE_BYTES);
        assert!(line.trim_end_matches("...").chars().all(|c| c == 'ä'));
    }
}
