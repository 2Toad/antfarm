<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { base, clock, dur, ms, rate, tilde } from "../lib/format";
  import { command } from "../lib/bridge";
  import type { AgentLine, CallLine, Class, Lane, Waiting } from "../lib/types";
  import Gauge from "../components/Gauge.svelte";
  import Glyph from "../components/Glyph.svelte";
  import Tool from "../components/Tool.svelte";

  interface FileItem { path: string; added: number; removed: number; tool: string; at: string | null }
  interface Body {
    id: string; name: string; project: string; project_key: string; branch: string | null; tree: string; source: string; pid: number | null;
    class: Class; status: string; for_secs: number | null; uptime_secs: number | null; active_secs: number | null; last_call_secs: number | null; active_partial: boolean;
    task: string | null; cost: string; usd_per_min: number; tokens_in: number; tokens_out: number; context_pct: number | null; calls: number;
    waiting: Waiting | null; agents: AgentLine[]; timeline_start: string | null; lanes: Lane[]; call_lines: CallLine[]; files: FileItem[];
    notify: { waiting: boolean; finished: boolean }; muted: boolean; snoozed: boolean;
  }

  const body = $derived(app.body<Body>("session"));
  let selected = $state(0);

  function copyCommand() {
    if (body?.waiting) command("copy", { text: body.waiting.request }).then((m) => app.say(m));
  }

  function setNotify(which: "waiting" | "finished", on: boolean) {
    if (!body) return;
    const notify = { ...body.notify, [which]: on };
    command("set_notify", { session: body.id, notify });
  }

  function openCall(c: CallLine) {
    if (body) app.go({ screen: "call", session: body.id, agent: c.agent, call: c.call });
  }

  export function key(e: KeyboardEvent): boolean {
    const n = body?.call_lines.length ?? 0;
    if (!n) return false;
    if (e.key === "ArrowDown" || e.key === "j") selected = Math.min(selected + 1, n - 1);
    else if (e.key === "ArrowUp" || e.key === "k") selected = Math.max(selected - 1, 0);
    else if (e.key === "Enter") openCall(body!.call_lines[Math.min(selected, n - 1)]!);
    else return false;
    document.querySelector(".crow.sel")?.scrollIntoView({ block: "nearest" });
    return true;
  }

  const ticks = $derived.by(() => {
    if (!body?.timeline_start || !app.view) return [] as string[];
    const a = new Date(body.timeline_start).getTime();
    const b = new Date(app.view.now).getTime();
    return [0, 0.2, 0.4, 0.6, 0.8].map((f) => new Date(a + (b - a) * f).toISOString());
  });
</script>

{#if body}
  <div class="session">
    <div class="main scroll">
      <div class="label">session · root agent</div>
      <div class="title">
        <h1>{body.name}</h1>
        <span>{body.source}{body.pid ? ` · pid ${body.pid}` : ""}</span>
        <span>{tilde(body.tree)}</span>
        <span>{body.branch ?? "-"}</span>
        <span>up {dur(body.uptime_secs)}</span>
        <span>{body.agents.length} agents · {body.calls} calls</span>
        <span>context {body.context_pct == null ? "-" : Math.round(body.context_pct) + "%"}</span>
      </div>

      {#if body.waiting}
        <section class="waiting" aria-label="Waiting for you">
          <div class="whead">
            <span><b>WAITING FOR YOU</b> {body.waiting.kind === "question" ? "question" : "permission request"}</span>
            <b class="age">{dur(body.waiting.age)}</b>
          </div>
          <div class="wbody">
            <p class="prose">
              {#if body.waiting.kind === "question"}The {body.waiting.agent} agent asked:
              {:else if body.waiting.tool}The {body.waiting.agent} agent wants to run a command it isn't allowed to run on its own:
              {:else}Claude Code reports that this session is waiting:{/if}
            </p>
            <pre class="cmd">{body.waiting.tool === "Bash" ? "$ " : ""}{body.waiting.request}</pre>
            <dl>
              {#if body.waiting.reason}<dt>reason</dt><dd>“{body.waiting.reason}”</dd>{/if}
              {#if body.waiting.touches.length}<dt>touches</dt><dd>{body.waiting.touches.join(" · ")}</dd>{/if}
              <dt>asked at</dt><dd>{clock(body.waiting.asked_at)}{body.waiting.after ? ` after ${body.waiting.after}` : ""}</dd>
            </dl>
            <div class="actions">
              <button class="primary" onclick={() => app.run("go_to_terminal", { session: body.id })}>Go to terminal · {body.source} <kbd>G</kbd></button>
              {#if body.waiting.tool}<button onclick={copyCommand}>Copy command <kbd>C</kbd></button>{/if}
              <button onclick={() => app.run("snooze", { session: body.id })}>Snooze 10 min <kbd>Z</kbd></button>
              <button onclick={() => app.run("mute", { session: body.id })}>{body.muted ? "Unmute" : "Mute this session"}</button>
              <span class="spacer"></span>
              <span class="dim">antfarm only watches. You answer in the terminal.</span>
            </div>
          </div>
        </section>
      {/if}

      <div class="pair">
        <section class="panel chamber">
          <div class="label">chamber · {body.agents.length} agents</div>
          {#each body.agents as a (a.id)}
            <button class="agent" style="padding-left:{8 + a.depth * 18}px" onclick={() => app.go({ screen: "agent", session: body.id, agent: a.id })}>
              <Glyph c={a.class} pulse /> <b>{a.name}</b> <span class="c-{a.class}">{a.state}</span> <span class="dim">{a.detail}</span>
            </button>
          {/each}
        </section>
        <section class="panel timeline">
          <div class="thead"><span class="label">timeline · {clock(body.timeline_start)} → now</span><button class="link" onclick={() => app.go({ screen: "lanes", id: body.id })}>Open swimlanes <kbd>L</kbd></button></div>
          {#each body.lanes as l}
            <div class="lane">
              <span class="lname" style="padding-left:{l.depth * 12}px">{l.depth ? "└ " : ""}{l.label}</span>
              <span class="track">
                {#each l.segments as [a, b, c]}
                  <span class="seg s-{c}" style="left:{a * 100}%;width:{Math.max(0.4, (b - a) * 100)}%"></span>
                {/each}
              </span>
            </div>
          {/each}
          <div class="ticks">{#each ticks as t}<span>{clock(t).slice(0, 5)}</span>{/each}</div>
          <div class="legend"><span class="c-Working">▬ working</span><span class="c-Finished">▬ finished</span><span class="c-Waiting">▬ waiting for you</span></div>
        </section>
      </div>

      <section class="panel calls">
        <div class="chead"><span class="label">tool calls · newest first</span><span class="dim">{body.calls} total · <kbd>↑↓</kbd> select · <kbd>⏎</kbd> open</span></div>
        {#each body.call_lines as c, i (c.agent + c.call)}
          <button class="crow" class:wait={c.waiting} class:sel={i === selected} onclick={() => { selected = i; openCall(c); }}>
            <span class="dim">{clock(c.at)}</span>
            <span class="who" class:c-Waiting={c.waiting}>{c.agent_name}</span>
            <Tool name={c.tool} />
            <span class="target">{c.target}</span>
            <span class="res" class:c-Waiting={c.waiting}>{c.waiting ? "asked" : c.status === "pending" ? "running" : c.status === "error" ? "✕ " + ms(c.elapsed) : ms(c.elapsed)}</span>
          </button>
        {/each}
      </section>
    </div>

    <aside class="rail scroll">
      <div class="label">status</div>
      <dl class="status">
        <dt>state</dt><dd class="c-{body.class}">{body.status}</dd>
        <dt>last motion</dt><dd>{body.last_call_secs == null ? "-" : dur(body.last_call_secs) + " ago"}</dd>
        <dt>active time</dt><dd>{dur(body.active_secs)}{#if body.active_partial} <span class="dim">· last 500 calls</span>{/if}</dd>
        <dt>tokens</dt><dd>{rate(body.tokens_in)} in · {rate(body.tokens_out)} out</dd>
        <dt>cost</dt><dd>{body.cost} so far · ${body.usd_per_min.toFixed(2)}/min</dd>
        <dt>context</dt><dd><Gauge value={body.context_pct} /></dd>
      </dl>
      <div class="label">carrying · files changed</div>
      {#each body.files.slice(0, 8) as f (f.path)}
        <div class="file"><span title={f.path}>{base(f.path)}</span><span><span class="add">+{f.added}</span> <span class="del">−{f.removed}</span></span></div>
      {:else}
        <p class="dim">No files edited yet.</p>
      {/each}
      {#if body.files.length > 8}<p class="dim">+ {body.files.length - 8} more files</p>{/if}
      {#if body.task}
        <div class="label">task</div>
        <p class="prose">“{body.task}”</p>
      {/if}
      <div class="label">notify me</div>
      <label><input type="checkbox" checked={body.notify.waiting} onchange={(e) => setNotify("waiting", e.currentTarget.checked)} /> when this session waits</label>
      <label><input type="checkbox" checked={body.notify.finished} onchange={(e) => setNotify("finished", e.currentTarget.checked)} /> when it finishes</label>
    </aside>
  </div>
{:else}
  <p class="dim pad">This session is gone.</p>
{/if}

<style>
  .session {
    display: grid;
    grid-template-columns: 1fr 380px;
    height: 100%;
  }
  .main {
    padding: 16px 20px;
    min-height: 0;
  }
  .title {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 18px;
    margin-bottom: 14px;
  }
  h1 {
    margin: 0;
    font-size: 26px;
  }
  .waiting {
    border: 2px solid var(--waiting);
    border-radius: 10px;
    margin-bottom: 16px;
    overflow: hidden;
  }
  .whead {
    display: flex;
    justify-content: space-between;
    background: var(--waiting);
    color: #111;
    padding: 10px 18px;
  }
  .age {
    font-size: 20px;
  }
  .wbody {
    padding: 14px 18px;
    background: var(--waiting-bg);
  }
  .cmd {
    font: 15px var(--mono);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 12px 14px;
    white-space: pre-wrap;
    word-break: break-all;
  }
  dl {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 4px;
    margin: 8px 0;
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
  }
  .actions {
    display: flex;
    gap: 10px;
    align-items: center;
    flex-wrap: wrap;
    margin-top: 10px;
  }
  .spacer {
    flex: 1;
  }
  .pair {
    display: grid;
    grid-template-columns: 2fr 3fr;
    gap: 14px;
    margin-bottom: 14px;
  }
  .chamber,
  .timeline,
  .calls {
    padding: 12px 14px;
  }
  .agent {
    display: block;
    width: 100%;
    text-align: left;
    border: 0;
    background: none;
    padding: 3px 8px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .thead,
  .chead {
    display: flex;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .link {
    border: 0;
    background: none;
    text-decoration: underline;
    padding: 0;
  }
  .lane {
    display: grid;
    grid-template-columns: 160px 1fr;
    align-items: center;
    height: 22px;
  }
  .lname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .track {
    position: relative;
    height: 10px;
  }
  .seg {
    position: absolute;
    top: 0;
    height: 10px;
    border-radius: 2px;
    background: var(--line);
  }
  .seg.s-Working {
    background: var(--working);
  }
  .seg.s-Waiting {
    background: var(--waiting);
  }
  .seg.s-Finished,
  .seg.s-Idle {
    background: var(--finished);
  }
  .ticks {
    display: flex;
    justify-content: space-between;
    margin: 6px 0 0 160px;
    color: var(--text-3);
  }
  .legend {
    display: flex;
    gap: 14px;
    margin-top: 6px;
  }
  .crow {
    display: grid;
    grid-template-columns: 70px 180px 80px 1fr 80px;
    gap: 10px;
    width: 100%;
    text-align: left;
    border: 0;
    border-top: 1px solid var(--line);
    border-radius: 0;
    background: none;
    padding: 6px 4px;
  }
  .crow.wait {
    background: var(--waiting-bg);
  }
  .who,
  .target {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .res {
    text-align: right;
  }
  .rail {
    border-left: 1px solid var(--line);
    padding: 16px 18px;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .status {
    grid-template-columns: 100px 1fr;
  }
  .file {
    display: flex;
    justify-content: space-between;
    gap: 10px;
  }
  .file span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pad {
    padding: 20px;
  }
</style>
