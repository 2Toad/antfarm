<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { base, clock, dur, tilde } from "../lib/format";
  import type { AgentLine, Class } from "../lib/types";
  import Ant from "../components/Ant.svelte";
  import Glyph from "../components/Glyph.svelte";

  interface Card { id: string; name: string; class: Class; state: string; for_secs: number | null; branch: string | null; tree: string; worktree: boolean; source: string; pid: number | null; carrying: number; added: number; removed: number; cost: string; usd_per_min: number; shares: string[]; agents: AgentLine[] }
  interface Touch { session: string; session_name: string; agent: string; tool: string; at: string | null; added: number; removed: number; worktree: boolean }
  interface File { path: string; last_at: string | null; touches: Touch[]; conflict: string | null }
  interface Branch { branch: string; tree: string; sessions: number; conflict: boolean }
  interface Body { key: string; name: string; sessions: Card[]; files: File[]; branches: Branch[]; agents: number; extra_worktrees: number; calls_per_min: number; cost: number; conflicts: number }

  const body = $derived(app.body<Body>("project"));
  const now = $derived(app.view?.now ?? new Date().toISOString());
  const ago = (at: string | null) => (at ? dur((new Date(now).getTime() - new Date(at).getTime()) / 1000) + " ago" : "");
</script>

{#if body}
  <div class="project">
    <div class="head">
      <h1>{body.name}</h1>
      <span class="dim">{tilde(body.key)}</span>
      <span>{body.sessions.length} sessions · {body.agents} agents</span>
      <span>{body.branches.length} branches{body.extra_worktrees ? ` · ${body.extra_worktrees} extra worktree${body.extra_worktrees > 1 ? "s" : ""}` : ""}</span>
      <span>{body.calls_per_min.toFixed(0)} calls/min</span>
      <span>${body.cost.toFixed(2)} so far</span>
      {#if body.conflicts}<span class="conflict-pill">⚠ CONFLICT</span><b>{body.conflicts} file{body.conflicts > 1 ? "s" : ""} edited by 2 sessions</b>{/if}
    </div>
    <div class="grid">
      <section class="surface">
        <div class="label">surface · sorted: needs you, then shared files</div>
        <div class="cards">
          {#each body.sessions as s (s.id)}
            <div class="colony">
              <button class="card s-{s.class}" onclick={() => app.go({ screen: "session", id: s.id })}>
                <div class="name">{s.name}</div>
                <div class="c-{s.class} state">{s.state} · {dur(s.for_secs)}</div>
                <dl>
                  <dt>branch</dt><dd>⑂ {s.branch ?? "-"}</dd>
                  <dt>tree</dt><dd>{tilde(s.tree)}</dd>
                  <dt>source</dt><dd>{s.source}{s.pid ? ` · pid ${s.pid}` : ""}</dd>
                  <dt>carrying</dt><dd>{s.carrying} files · <span class="add">+{s.added}</span> <span class="del">−{s.removed}</span></dd>
                  <dt>cost</dt><dd>{s.cost} · ${s.usd_per_min.toFixed(2)}/min</dd>
                </dl>
                {#if s.shares.length}<div class="shares">⚠ shares {s.shares.join(", ")}</div>{:else if s.worktree}<div class="dim">separate worktree · no overlap</div>{/if}
                <div class="ant"><Ant c={s.class} size={20} /></div>
              </button>
              {#if s.agents.length}
                <div class="subs">
                  {#each s.agents.slice(0, 6) as a (a.id)}
                    <button class="sub s-{a.class}" onclick={() => app.go({ screen: "agent", session: s.id, agent: a.id })}>
                      <b>{a.name}</b>
                      <span class="c-{a.class}">{a.state} · {a.detail}</span>
                    </button>
                  {/each}
                </div>
              {:else}
                <div class="dim nosubs">no sub-agents</div>
              {/if}
            </div>
          {/each}
        </div>
      </section>
      <aside class="side">
        <div class="shead"><span class="label">files touched · last 10 min</span><span class="dim">{body.files.length} files · edits only</span></div>
        {#if !body.files.length}<p class="dim pad">No edits in the last ten minutes.</p>{/if}
        {#each body.files as f (f.path)}
          <div class="file" class:hot={f.conflict}>
            <div class="frow"><b title={f.path}>{base(f.path)}</b>{#if f.conflict}<span class="conflict-pill">⚠ CONFLICT</span>{/if}<span class="dim">{ago(f.last_at)}</span></div>
            {#each f.touches as t}
              <div class="touch">
                <Glyph c={body.sessions.find((s) => s.id === t.session)?.class ?? "Finished"} />
                <span>{t.session_name}</span>
                <span class="dim">{clock(t.at)}</span>
                <span class="add">+{t.added}</span> <span class="del">−{t.removed}</span>
                {#if t.worktree}<span class="dim">worktree</span>{/if}
              </div>
            {/each}
            {#if f.conflict}
              <p class="prose explain">{f.conflict}</p>
              <div class="actions">
                {#each [...new Map(f.touches.map((t) => [t.session, t.session_name]))] as [id, name] (id)}
                  <button onclick={() => app.go({ screen: "session", id })}>Open {name}</button>
                {/each}
                <button onclick={() => app.run("go_to_terminal", { session: f.touches[0]!.session })}>Go to terminal · {f.touches[0]!.session_name}</button>
              </div>
            {/if}
          </div>
        {/each}
        <div class="shead"><span class="label">branches &amp; worktrees</span></div>
        {#each body.branches as b}
          <div class="branch"><span>⑂ {b.branch}</span><span class="dim">{tilde(b.tree)}</span><b>{b.sessions} session{b.sessions > 1 ? "s" : ""}{b.conflict ? " ⚠" : ""}</b></div>
        {/each}
      </aside>
    </div>
  </div>
{:else}
  <p class="dim pad">This project has no live sessions.</p>
{/if}

<style>
  .project {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 22px;
    padding: 14px 20px;
    border-bottom: 1px solid var(--line);
  }
  h1 {
    margin: 0;
    font-size: 22px;
  }
  .conflict-pill {
    background: var(--text);
    color: var(--bg);
    font-weight: 700;
    padding: 1px 8px;
    border-radius: 4px;
    font-size: 11px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 440px;
    flex: 1;
    min-height: 0;
  }
  .surface {
    padding: 14px 20px;
    overflow: auto;
    background-image: radial-gradient(var(--line) 0.6px, transparent 0.6px);
    background-size: 14px 14px;
  }
  .cards {
    display: flex;
    gap: 24px;
    flex-wrap: wrap;
    margin-top: 18px;
  }
  .colony {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 340px;
  }
  .card {
    position: relative;
    text-align: left;
    border-radius: 24px;
    padding: 18px 20px;
    border: 2px solid var(--line);
    background: var(--panel);
  }
  .s-Working {
    border-color: var(--working);
    background: var(--working-bg);
  }
  .s-Waiting {
    border: 3px solid var(--waiting);
    background: var(--waiting-bg);
    box-shadow: 0 0 0 5px var(--waiting-bg);
  }
  .s-Finished,
  .s-Idle {
    border-color: var(--finished);
  }
  .s-Still,
  .s-Exited {
    border-style: dashed;
    border-color: var(--still);
  }
  .name {
    font-size: 17px;
    font-weight: 700;
  }
  .state {
    margin-bottom: 8px;
  }
  dl {
    display: grid;
    grid-template-columns: 80px 1fr;
    margin: 0;
    gap: 2px;
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .shares {
    margin-top: 8px;
    font-weight: 700;
  }
  .ant {
    position: absolute;
    right: 18px;
    bottom: 12px;
  }
  .subs {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }
  .sub {
    min-width: 0;
    overflow: hidden;
    text-align: left;
    border-radius: 14px;
    padding: 6px 12px;
    display: flex;
    flex-direction: column;
    border: 1.5px solid var(--line);
    background: var(--panel);
  }
  .sub b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .nosubs {
    text-align: center;
  }
  .side {
    border-left: 1px solid var(--line);
    overflow: auto;
  }
  .shead {
    display: flex;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
  }
  .file {
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
  }
  .file.hot {
    outline: 1px solid var(--text-3);
    background: var(--raise);
  }
  .frow {
    display: flex;
    gap: 10px;
    align-items: center;
  }
  .frow b {
    flex: 1;
  }
  .touch {
    display: flex;
    gap: 10px;
    padding-left: 4px;
  }
  .explain {
    margin: 8px 0;
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .branch {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: 12px;
    padding: 6px 16px;
    white-space: nowrap;
  }
  .branch span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pad {
    padding: 12px 16px;
  }
</style>
