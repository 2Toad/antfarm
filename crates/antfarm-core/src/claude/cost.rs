//! Finds the last `cost-state` line of a main transcript so the engine can anchor the
//! session's cost on Claude Code's own total instead of pricing the whole file.
//!
//! This is source-side I/O. The engine never calls it; a source does on attach and
//! sends the result as `Input::CostAnchor` before that transcript's lines.

use std::io;
use std::path::Path;

use super::parse::{Parsed, parse_line};
use crate::model::Time;
use crate::tail::find_last_line_containing;

/// Scans backward over at most `max_scan` bytes for the last `cost-state` line and
/// returns its `totalCostUSD`, byte offset, and timestamp (which current builds omit).
/// `None` when there is no such line in range or it does not parse.
pub fn cost_anchor(path: &Path, max_scan: u64) -> io::Result<Option<(f64, u64, Option<Time>)>> {
    let Some(raw) = find_last_line_containing(path, br#""type":"cost-state""#, max_scan)? else {
        return Ok(None);
    };
    let Ok(Parsed::CostState(c)) = parse_line(&raw.bytes, raw.offset) else {
        return Ok(None);
    };
    Ok(c.total_cost_usd.map(|usd| (usd, raw.offset, c.timestamp)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn finds_the_last_cost_state() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("main.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        let first = r#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        let cs1 = r#"{"type":"cost-state","sessionId":"s","totalCostUSD":1.5}"#;
        let mid = r#"{"type":"assistant","message":{"id":"m","content":[]}}"#;
        let cs2 =
            r#"{"type":"cost-state","sessionId":"s","totalCostUSD":2.25,"totalLinesAdded":3}"#;
        let last = r#"{"type":"user","message":{"role":"user","content":"a \"type\":\"cost-state\" lookalike inside a string"}}"#;
        for l in [first, cs1, mid, cs2, last] {
            writeln!(f, "{l}").unwrap();
        }
        drop(f);
        let cs2_offset = (first.len() + 1 + cs1.len() + 1 + mid.len() + 1) as u64;
        // The lookalike in the last line has escaped quotes, so the byte needle skips it
        // and the real last cost-state is found.
        assert_eq!(
            cost_anchor(&p, u64::MAX).unwrap(),
            Some((2.25, cs2_offset, None))
        );
        // A scan window too short to reach it finds nothing.
        assert_eq!(cost_anchor(&p, 10).unwrap(), None);
        // A window reaching back only as far as the last cost-state still finds it.
        assert_eq!(
            cost_anchor(&p, (last.len() + 1 + cs2.len() + 1) as u64).unwrap(),
            Some((2.25, cs2_offset, None))
        );
    }

    #[test]
    fn no_cost_state_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("main.jsonl");
        std::fs::write(&p, "{\"type\":\"user\"}\n").unwrap();
        assert_eq!(cost_anchor(&p, u64::MAX).unwrap(), None);
    }
}
