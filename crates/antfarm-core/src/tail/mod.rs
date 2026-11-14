//! Follows a growing JSONL file by byte offset.
//!
//! Attach reads only the last `max_tail` bytes, starting at the first line boundary in
//! that window. Each poll returns the complete lines appended since the last poll; a
//! trailing partial line is held back until its newline arrives. A file that shrinks or
//! is replaced is re-read from the start.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::claude::parse::RawLine;

pub mod watcher;

#[derive(Debug, Clone, Copy)]
pub struct TailConfig {
    /// How much history to read on attach.
    pub max_tail: u64,
    /// Lines longer than this are dropped and counted.
    pub max_line: usize,
    pub read_chunk: usize,
}

impl Default for TailConfig {
    fn default() -> Self {
        Self {
            max_tail: 16 << 20,
            max_line: 64 << 20,
            read_chunk: 1 << 20,
        }
    }
}

#[derive(Debug, Default)]
pub struct Poll {
    pub lines: Vec<RawLine>,
    /// The file shrank or was replaced; `lines` starts from the new beginning.
    pub truncated: bool,
    /// Offset just after the last complete line returned.
    pub end_offset: u64,
}

#[derive(Debug)]
pub struct Tailer {
    path: PathBuf,
    file: File,
    identity: FileId,
    config: TailConfig,
    /// Bytes read from the file so far.
    read_to: u64,
    /// Bytes of an incomplete line, starting at `pending_start`.
    pending: Vec<u8>,
    pending_start: u64,
    /// Discarding bytes until the next newline: mid-line attach or an oversized line.
    skipping: bool,
    truncated_before: Option<u64>,
    oversized: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    dev: u64,
    ino: u64,
}

fn file_id(meta: &std::fs::Metadata) -> FileId {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        FileId {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        FileId { dev: 0, ino: 0 }
    }
}

impl Tailer {
    pub fn attach(path: impl AsRef<Path>, config: TailConfig) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let meta = file.metadata()?;
        let len = meta.len();
        let start = len.saturating_sub(config.max_tail);
        let mut t = Self {
            path,
            file,
            identity: file_id(&meta),
            config,
            read_to: start,
            pending: Vec::new(),
            pending_start: start,
            skipping: start > 0,
            truncated_before: None,
            oversized: 0,
        };
        if start > 0 {
            t.skip_to_line_start(len)?;
            t.truncated_before = Some(t.pending_start);
        }
        Ok(t)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Offset of the first byte read, when history before it was skipped.
    pub fn truncated_before(&self) -> Option<u64> {
        self.truncated_before
    }

    /// Lines dropped for exceeding `max_line`.
    pub fn oversized(&self) -> u64 {
        self.oversized
    }

    /// Offset just after the last complete line returned.
    pub fn offset(&self) -> u64 {
        self.pending_start
    }

    /// Follows the file to `path` after it moved, carrying on from the same offset. A move
    /// keeps the content, so the offset still lines up; a file there that is shorter than
    /// what was read is treated as replaced on the next poll and read from the start.
    pub fn relocate(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let file = File::open(path.as_ref())?;
        self.identity = file_id(&file.metadata()?);
        self.file = file;
        self.path = path.as_ref().to_path_buf();
        Ok(())
    }

    /// True when the file has grown past what was read, or changed identity.
    pub fn has_more(&self) -> io::Result<bool> {
        let meta = std::fs::metadata(&self.path)?;
        Ok(meta.len() != self.read_to || file_id(&meta) != self.identity)
    }

    /// Advances past the partial line at the attach point.
    fn skip_to_line_start(&mut self, len: u64) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(self.read_to))?;
        let mut buf = vec![0u8; self.config.read_chunk.min(64 << 10)];
        while self.read_to < len {
            let want = buf.len().min((len - self.read_to) as usize);
            let n = self.file.read(&mut buf[..want])?;
            if n == 0 {
                break;
            }
            if let Some(i) = memchr::memchr(b'\n', &buf[..n]) {
                self.read_to += (i + 1) as u64;
                self.pending_start = self.read_to;
                self.skipping = false;
                return Ok(());
            }
            self.read_to += n as u64;
        }
        // No line boundary yet: keep skipping on the next poll.
        self.pending_start = self.read_to;
        Ok(())
    }

    pub fn poll(&mut self) -> io::Result<Poll> {
        let mut out = Poll::default();
        let meta = std::fs::metadata(&self.path)?;
        let id = file_id(&meta);
        if id != self.identity || meta.len() < self.read_to {
            self.file = File::open(&self.path)?;
            self.identity = file_id(&self.file.metadata()?);
            self.read_to = 0;
            self.pending.clear();
            self.pending_start = 0;
            self.skipping = false;
            self.truncated_before = None;
            out.truncated = true;
        }
        let len = self.file.metadata()?.len();
        if len > self.read_to {
            self.file.seek(SeekFrom::Start(self.read_to))?;
            let mut buf = vec![0u8; self.config.read_chunk];
            while self.read_to < len {
                let want = buf.len().min((len - self.read_to) as usize);
                let n = self.file.read(&mut buf[..want])?;
                if n == 0 {
                    break;
                }
                self.consume(&buf[..n], &mut out.lines);
            }
        }
        out.end_offset = self.pending_start;
        Ok(out)
    }

    fn consume(&mut self, mut chunk: &[u8], lines: &mut Vec<RawLine>) {
        while !chunk.is_empty() {
            match memchr::memchr(b'\n', chunk) {
                Some(i) => {
                    let end = self.read_to + i as u64 + 1;
                    if self.skipping {
                        self.skipping = false;
                    } else {
                        let mut bytes = std::mem::take(&mut self.pending);
                        bytes.extend_from_slice(&chunk[..i]);
                        if bytes.len() > self.config.max_line {
                            self.oversized += 1;
                        } else {
                            lines.push(RawLine {
                                offset: self.pending_start,
                                bytes,
                            });
                        }
                    }
                    self.pending_start = end;
                    self.read_to = end;
                    chunk = &chunk[i + 1..];
                }
                None => {
                    if !self.skipping {
                        self.pending.extend_from_slice(chunk);
                        if self.pending.len() > self.config.max_line {
                            self.pending = Vec::new();
                            self.oversized += 1;
                            self.skipping = true;
                        }
                    }
                    self.read_to += chunk.len() as u64;
                    chunk = &[];
                }
            }
        }
    }
}

/// The last `max` bytes of a file as text, starting at a line boundary when cut, for
/// showing a background command's output. Invalid UTF-8 is replaced.
pub fn tail_text(path: &Path, max: u64) -> io::Result<String> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(max);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.take(max).read_to_end(&mut buf)?;
    let body = if start > 0 {
        memchr::memchr(b'\n', &buf)
            .map(|i| &buf[i + 1..])
            .unwrap_or(&buf[..])
    } else {
        &buf[..]
    };
    Ok(String::from_utf8_lossy(body).into_owned())
}

/// Finds the last line containing `needle`, scanning backward from the end of the file
/// over at most `max_scan` bytes. Returns the line's offset and bytes.
pub fn find_last_line_containing(
    path: &Path,
    needle: &[u8],
    max_scan: u64,
) -> io::Result<Option<RawLine>> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let floor = len.saturating_sub(max_scan);
    let block = 4u64 << 20;
    let finder = memchr::memmem::FinderRev::new(needle);
    let mut hi = len;
    // Overlap blocks by the needle length so a match spanning a boundary is seen.
    while hi > floor {
        let lo = hi.saturating_sub(block).max(floor);
        let read_hi = (hi + needle.len() as u64).min(len);
        let mut buf = vec![0u8; (read_hi - lo) as usize];
        f.seek(SeekFrom::Start(lo))?;
        f.read_exact(&mut buf)?;
        if let Some(pos) = finder.rfind(&buf) {
            let abs = lo + pos as u64;
            return read_line_around(&mut f, abs, len).map(Some);
        }
        hi = lo;
    }
    Ok(None)
}

fn read_line_around(f: &mut File, pos: u64, len: u64) -> io::Result<RawLine> {
    let back = 1u64 << 20;
    let lo = pos.saturating_sub(back);
    let mut buf = vec![0u8; (pos - lo) as usize];
    f.seek(SeekFrom::Start(lo))?;
    f.read_exact(&mut buf)?;
    let start = memchr::memrchr(b'\n', &buf)
        .map(|i| lo + i as u64 + 1)
        .unwrap_or(lo);
    f.seek(SeekFrom::Start(start))?;
    let mut line = Vec::new();
    let mut chunk = vec![0u8; 64 << 10];
    let mut at = start;
    while at < len {
        let n = f.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if let Some(i) = memchr::memchr(b'\n', &chunk[..n]) {
            line.extend_from_slice(&chunk[..i]);
            break;
        }
        line.extend_from_slice(&chunk[..n]);
        at += n as u64;
    }
    Ok(RawLine {
        offset: start,
        bytes: line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn cfg(max_tail: u64) -> TailConfig {
        TailConfig {
            max_tail,
            max_line: 1024,
            read_chunk: 7,
        }
    }

    fn texts(p: &Poll) -> Vec<String> {
        p.lines
            .iter()
            .map(|l| String::from_utf8(l.bytes.clone()).unwrap())
            .collect()
    }

    fn append(path: &Path, s: &str) {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .unwrap();
        f.write_all(s.as_bytes()).unwrap();
    }

    #[test]
    fn partial_line_is_held_until_complete() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        append(&p, "one\ntwo\nthr");
        let mut t = Tailer::attach(&p, cfg(1 << 20)).unwrap();
        let a = t.poll().unwrap();
        assert_eq!(texts(&a), ["one", "two"]);
        assert_eq!(a.lines[1].offset, 4);
        assert_eq!(a.end_offset, 8);
        assert!(t.poll().unwrap().lines.is_empty());
        append(&p, "ee\nfour\n");
        let b = t.poll().unwrap();
        assert_eq!(texts(&b), ["three", "four"]);
        assert_eq!(b.lines[0].offset, 8);
        assert_eq!(t.offset(), 19);
        assert!(!t.has_more().unwrap());
    }

    #[test]
    fn attach_back_seeks_to_a_line_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        append(&p, "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\n");
        // 15 bytes back from 33 lands inside the b line.
        let mut t = Tailer::attach(&p, cfg(15)).unwrap();
        assert_eq!(t.truncated_before(), Some(22));
        let a = t.poll().unwrap();
        assert_eq!(texts(&a), ["cccccccccc"]);
        assert_eq!(a.lines[0].offset, 22);
    }

    #[test]
    fn attach_with_no_boundary_in_window_skips_the_partial_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        append(&p, "aaaa\nbbbbbbbbbbbbbbbbbbbb");
        let mut t = Tailer::attach(&p, cfg(5)).unwrap();
        assert!(t.poll().unwrap().lines.is_empty());
        append(&p, "bbb\nnext\n");
        assert_eq!(texts(&t.poll().unwrap()), ["next"]);
    }

    #[test]
    fn a_moved_file_is_followed_from_the_same_offset() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        append(&p, "one\ntwo\n");
        let mut t = Tailer::attach(&p, cfg(1 << 20)).unwrap();
        assert_eq!(texts(&t.poll().unwrap()), ["one", "two"]);
        let q = dir.path().join("moved.jsonl");
        std::fs::rename(&p, &q).unwrap();
        append(&q, "three\n");
        assert!(t.poll().is_err(), "the old path is gone");
        t.relocate(&q).unwrap();
        let after = t.poll().unwrap();
        assert!(!after.truncated);
        assert_eq!(texts(&after), ["three"]);
        assert_eq!(t.path(), q);
    }

    #[test]
    fn a_shorter_file_at_the_new_path_is_read_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        append(&p, "one\ntwo\n");
        let mut t = Tailer::attach(&p, cfg(1 << 20)).unwrap();
        t.poll().unwrap();
        let q = dir.path().join("other.jsonl");
        append(&q, "x\n");
        t.relocate(&q).unwrap();
        let after = t.poll().unwrap();
        assert!(after.truncated);
        assert_eq!(texts(&after), ["x"]);
    }

    #[test]
    fn shrinking_file_is_reread_from_start() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        append(&p, "one\ntwo\n");
        let mut t = Tailer::attach(&p, cfg(1 << 20)).unwrap();
        t.poll().unwrap();
        std::fs::write(&p, "x\n").unwrap();
        let a = t.poll().unwrap();
        assert!(a.truncated);
        assert_eq!(texts(&a), ["x"]);
    }

    #[test]
    fn replaced_file_is_reread_from_start() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        append(&p, "one\n");
        let mut t = Tailer::attach(&p, cfg(1 << 20)).unwrap();
        t.poll().unwrap();
        let tmp = dir.path().join("new");
        std::fs::write(&tmp, "alpha\nbeta\n").unwrap();
        std::fs::rename(&tmp, &p).unwrap();
        assert!(t.has_more().unwrap());
        let a = t.poll().unwrap();
        assert!(a.truncated);
        assert_eq!(texts(&a), ["alpha", "beta"]);
    }

    #[test]
    fn oversized_line_is_dropped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let big = "z".repeat(3000);
        append(&p, &format!("ok\n{big}\nafter\n"));
        let mut t = Tailer::attach(&p, cfg(1 << 20)).unwrap();
        let a = t.poll().unwrap();
        assert_eq!(texts(&a), ["ok", "after"]);
        assert_eq!(a.lines[1].offset, 3 + 3001);
        assert_eq!(t.oversized(), 1);
    }

    #[test]
    fn tail_text_starts_on_a_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out");
        append(&p, "first line\nsecond line\nthird\n");
        assert_eq!(
            tail_text(&p, 1 << 20).unwrap(),
            "first line\nsecond line\nthird\n"
        );
        assert_eq!(tail_text(&p, 14).unwrap(), "third\n");
        assert!(tail_text(&dir.path().join("missing"), 10).is_err());
    }

    #[test]
    fn finds_last_matching_line_backward() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        append(
            &p,
            "{\"type\":\"cost-state\",\"n\":1}\nfiller\n{\"type\":\"cost-state\",\"n\":2}\ntail\n",
        );
        let l = find_last_line_containing(&p, br#""type":"cost-state""#, 1 << 20)
            .unwrap()
            .unwrap();
        assert_eq!(l.bytes, br#"{"type":"cost-state","n":2}"#);
        assert_eq!(l.offset, 35);
        assert!(
            find_last_line_containing(&p, b"missing", 1 << 20)
                .unwrap()
                .is_none()
        );
        assert!(
            find_last_line_containing(&p, br#""n":1"#, 20)
                .unwrap()
                .is_none()
        );
    }
}
