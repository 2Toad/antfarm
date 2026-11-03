//! `antfarm-scrub` turns a real Claude Code transcript into an anonymized fixture
//! directory for the public repo.
//!
//! ```text
//! antfarm-scrub <transcript.jsonl> --out fixtures/real/<name> [--tail-bytes 4M]
//!     [--agents 6] [--sub-tail-bytes 1M] [--registry <pid>.json] [--verify]
//!     [--include-agent <id>]... [--drop-keys k1,k2]
//! ```
//!
//! Selection reads only the last `--tail-bytes` of the transcript (through the same
//! tailer the live source uses), keeps the line types antfarm parses, and pulls in the
//! sub-agents those lines mention and the workflows those touch. Every output file is
//! then scrubbed by [`scrub`]. `--verify` feeds the unscrubbed selection (in memory) and
//! the written output through the parser and engine and requires identical results.

mod json;
mod scrub;
mod select;
mod verify;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use antfarm_core::input::AgentRef;
use anyhow::{Context, Result};
use clap::Parser;

use json::Json;

/// Top-level keys left out of every line. They are opaque to antfarm and duplicate data
/// scrubbed elsewhere (`wireToolInputs` repeats each `tool_use.input`; the classifier
/// fields are server-side request blobs). Dropping them keeps fixtures small.
const DEFAULT_DROP_KEYS: &str = "wireToolInputs,serverClassifierContext,serverClassifierRequest";

const MAX_WORKFLOWS: usize = 3;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Turn a Claude Code transcript into an anonymized fixture"
)]
struct Cli {
    /// Main transcript, `~/.claude/projects/<slug>/<sessionId>.jsonl`.
    transcript: PathBuf,
    /// Fixture directory to write.
    #[arg(long)]
    out: PathBuf,
    /// How much of the end of the main transcript to keep (K, M, G suffixes).
    #[arg(long, default_value = "4M", value_parser = parse_size)]
    tail_bytes: u64,
    /// Most sub-agents to keep, most recent first.
    #[arg(long, default_value_t = 6)]
    agents: usize,
    /// How much of the end of each sub-agent transcript to keep.
    #[arg(long, default_value = "1M", value_parser = parse_size)]
    sub_tail_bytes: u64,
    /// A `~/.claude/sessions/<pid>.json` entry to copy, scrubbed, as `registry.json`.
    #[arg(long)]
    registry: Option<PathBuf>,
    /// Re-parse the unscrubbed selection and the output and require identical results.
    #[arg(long)]
    verify: bool,
    /// Force a sub-agent (and its parent chain) into the fixture. Counts toward --agents.
    #[arg(long = "include-agent")]
    include_agents: Vec<String>,
    /// Top-level transcript keys to leave out of every line.
    #[arg(long, value_delimiter = ',', default_value = DEFAULT_DROP_KEYS)]
    drop_keys: Vec<String>,
}

fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mult) = match s.chars().last() {
        Some('K' | 'k') => (&s[..s.len() - 1], 1u64 << 10),
        Some('M' | 'm') => (&s[..s.len() - 1], 1u64 << 20),
        Some('G' | 'g') => (&s[..s.len() - 1], 1u64 << 30),
        _ => (s, 1),
    };
    num.parse::<u64>()
        .map(|n| n * mult)
        .map_err(|e| format!("{s}: {e}"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let drop_keys: Vec<String> = cli
        .drop_keys
        .iter()
        .filter(|k| !k.is_empty())
        .cloned()
        .collect();
    let opts = select::Options {
        tail_bytes: cli.tail_bytes,
        sub_tail_bytes: cli.sub_tail_bytes,
        agents: cli.agents,
        workflows: MAX_WORKFLOWS,
        include_agents: &cli.include_agents,
    };
    let selection = select::select(&cli.transcript, &opts)?;
    let registry = match &cli.registry {
        Some(p) => Some(
            Json::parse(&fs::read(p).with_context(|| format!("read {}", p.display()))?)
                .with_context(|| format!("parse {}", p.display()))?,
        ),
        None => None,
    };

    // Unscrubbed bytes stay in memory for --verify; they are never written.
    let drop = |mut w: select::Window| -> select::Window {
        for l in &mut w.lines {
            for k in &drop_keys {
                l.remove(k);
            }
        }
        w
    };
    let main = drop(selection.main);
    let subagents: Vec<select::Subagent> = selection
        .subagents
        .into_iter()
        .map(|mut s| {
            s.window = drop(s.window);
            s
        })
        .collect();
    let main_original = main.bytes();
    let sub_original: Vec<Vec<u8>> = subagents.iter().map(|s| s.window.bytes()).collect();

    let out = &cli.out;
    prepare_out_dir(out)?;
    let mut line_counts: Vec<(String, usize)> = Vec::new();

    let main_path = out.join("main.jsonl");
    write_jsonl(&main_path, &main.lines, &drop_keys)?;
    line_counts.push(("main.jsonl".into(), main.lines.len()));

    let mut agent_ids = Vec::new();
    for s in &subagents {
        let rel = format!("subagents/agent-{}.jsonl", s.id);
        write_jsonl(&out.join(&rel), &s.window.lines, &drop_keys)?;
        line_counts.push((rel, s.window.lines.len()));
        if let Some(meta) = &s.meta {
            let mut meta = meta.clone();
            scrub::scrub_meta(&mut meta);
            fs::write(
                out.join(format!("subagents/agent-{}.meta.json", s.id)),
                meta.to_vec(),
            )?;
        }
        agent_ids.push(s.id.clone());
    }
    let mut workflow_ids = Vec::new();
    for w in &selection.workflows {
        let mut json = w.json.clone();
        scrub::scrub_workflow(&mut json);
        fs::write(
            out.join(format!("workflows/{}.json", w.id)),
            json.to_pretty() + "\n",
        )?;
        workflow_ids.push(w.id.clone());
    }
    if let Some(mut reg) = registry {
        scrub::scrub_meta(&mut reg);
        fs::write(out.join("registry.json"), reg.to_pretty() + "\n")?;
    }

    let mut versions: BTreeSet<String> = main.versions.clone();
    for s in &subagents {
        versions.extend(s.window.versions.iter().cloned());
    }
    let toml = fixture_toml(&FixtureInfo {
        versions: &versions,
        original_len: selection.original_len,
        truncated_before: main.truncated_before,
        tail_bytes: cli.tail_bytes,
        sub_tail_bytes: cli.sub_tail_bytes,
        drop_keys: &drop_keys,
        dropped: main.dropped + subagents.iter().map(|s| s.window.dropped).sum::<u64>(),
        line_counts: &line_counts,
        agents: &subagents,
        workflow_ids: &workflow_ids,
        registry: cli.registry.is_some(),
    });
    fs::write(out.join("FIXTURE.toml"), toml)?;

    let total: u64 = walk_size(out)?;
    println!(
        "wrote {}: {} main lines, {} sub-agents, {} workflows, {} bytes total",
        out.display(),
        main.lines.len(),
        subagents.len(),
        workflow_ids.len(),
        total
    );
    for s in &subagents {
        println!(
            "  agent {} depth {} parent {} lines {} truncated_before {:?}",
            s.id,
            s.spawn_depth,
            s.parent.as_deref().unwrap_or("-"),
            s.window.lines.len(),
            s.window.truncated_before
        );
    }
    if !main.ignored.is_empty() {
        let mut ig: Vec<_> = main.ignored.iter().collect();
        ig.sort_by(|a, b| b.1.cmp(a.1));
        let list: Vec<String> = ig.iter().map(|(k, v)| format!("{k} {v}")).collect();
        println!("  dropped line types in main: {}", list.join(", "));
    }

    if cli.verify {
        let session = cli
            .transcript
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session")
            .to_owned();
        let mut original = vec![verify::Feed {
            agent: AgentRef::Root,
            bytes: &main_original,
            truncated_before: main.truncated_before,
            path: None,
        }];
        let mut scrubbed = vec![verify::Feed {
            agent: AgentRef::Root,
            bytes: &[],
            truncated_before: main.truncated_before,
            path: Some(&main_path),
        }];
        let sub_paths: Vec<PathBuf> = subagents
            .iter()
            .map(|s| out.join(format!("subagents/agent-{}.jsonl", s.id)))
            .collect();
        for (i, s) in subagents.iter().enumerate() {
            let agent = AgentRef::Sub(s.id.as_str().into());
            original.push(verify::Feed {
                agent: agent.clone(),
                bytes: &sub_original[i],
                truncated_before: s.window.truncated_before,
                path: None,
            });
            scrubbed.push(verify::Feed {
                agent,
                bytes: &[],
                truncated_before: s.window.truncated_before,
                path: Some(&sub_paths[i]),
            });
        }
        let a = verify::run(&session, &original)?;
        let b = verify::run(&session, &scrubbed)?;
        println!("verify original: {}", verify::summary(&a));
        println!("verify scrubbed: {}", verify::summary(&b));
        verify::compare(&a, &b)?;
        println!("verify ok: parse counters, open calls, agents, usage, cost, and context match");
    }
    Ok(())
}

/// Clears the files a previous run wrote so a re-run never leaves stale output behind.
fn prepare_out_dir(out: &Path) -> Result<()> {
    fs::create_dir_all(out)?;
    for name in ["main.jsonl", "registry.json", "FIXTURE.toml"] {
        let p = out.join(name);
        if p.exists() {
            fs::remove_file(&p)?;
        }
    }
    for dir in ["subagents", "workflows"] {
        let p = out.join(dir);
        if p.exists() {
            fs::remove_dir_all(&p)?;
        }
        fs::create_dir_all(&p)?;
    }
    Ok(())
}

fn write_jsonl(path: &Path, lines: &[Json], drop_keys: &[String]) -> Result<()> {
    let mut buf = Vec::new();
    for l in lines {
        let mut l = l.clone();
        scrub::scrub_line(&mut l, drop_keys);
        buf.extend(l.to_vec());
        buf.push(b'\n');
    }
    fs::write(path, buf).with_context(|| format!("write {}", path.display()))
}

fn walk_size(dir: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        total += if p.is_dir() {
            walk_size(&p)?
        } else {
            fs::metadata(&p)?.len()
        };
    }
    Ok(total)
}

struct FixtureInfo<'a> {
    versions: &'a BTreeSet<String>,
    original_len: u64,
    truncated_before: Option<u64>,
    tail_bytes: u64,
    sub_tail_bytes: u64,
    drop_keys: &'a [String],
    dropped: u64,
    line_counts: &'a [(String, usize)],
    agents: &'a [select::Subagent],
    workflow_ids: &'a [String],
    registry: bool,
}

fn toml_list(items: impl Iterator<Item = String>) -> String {
    let quoted: Vec<String> = items.map(|s| format!("{s:?}")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Writes the fixture's provenance. Nothing here names the source path, slug, or project.
fn fixture_toml(info: &FixtureInfo) -> String {
    let mut t = String::new();
    let _ = writeln!(
        t,
        "# Scrubbed Claude Code transcript fixture. Produced by antfarm-scrub; every"
    );
    let _ = writeln!(
        t,
        "# user-visible string is replaced by same-length placeholder text."
    );
    let _ = writeln!(
        t,
        "claude_code_versions = {}",
        toml_list(info.versions.iter().cloned())
    );
    let _ = writeln!(
        t,
        "captured = {:?}",
        chrono::Utc::now().date_naive().to_string()
    );
    let _ = writeln!(t, "original_main_bytes = {}", info.original_len);
    match info.truncated_before {
        Some(n) => {
            let _ = writeln!(t, "truncated_before = {n}");
        }
        None => {
            let _ = writeln!(t, "# truncated_before: whole file kept");
        }
    }
    let _ = writeln!(t, "tail_bytes = {}", info.tail_bytes);
    let _ = writeln!(t, "sub_tail_bytes = {}", info.sub_tail_bytes);
    let _ = writeln!(
        t,
        "dropped_keys = {}",
        toml_list(info.drop_keys.iter().cloned())
    );
    let _ = writeln!(t, "unparseable_lines_dropped = {}", info.dropped);
    let _ = writeln!(t, "registry = {}", info.registry);
    let _ = writeln!(
        t,
        "subagents = {}",
        toml_list(info.agents.iter().map(|a| a.id.clone()))
    );
    let _ = writeln!(
        t,
        "workflows = {}",
        toml_list(info.workflow_ids.iter().cloned())
    );
    let _ = writeln!(t, "\n[lines]");
    for (name, n) in info.line_counts {
        let _ = writeln!(t, "{name:?} = {n}");
    }
    let _ = writeln!(t, "\n[subagent_tails]");
    for a in info.agents {
        let _ = writeln!(
            t,
            "{:?} = {{ truncated_before = {}, spawn_depth = {}, parent = {:?} }}",
            a.id,
            a.window.truncated_before.unwrap_or(0),
            a.spawn_depth,
            a.parent.as_deref().unwrap_or("")
        );
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse() {
        assert_eq!(parse_size("4M").unwrap(), 4 << 20);
        assert_eq!(parse_size("256K").unwrap(), 256 << 10);
        assert_eq!(parse_size("1g").unwrap(), 1 << 30);
        assert_eq!(parse_size("123").unwrap(), 123);
        assert!(parse_size("x").is_err());
    }
}
