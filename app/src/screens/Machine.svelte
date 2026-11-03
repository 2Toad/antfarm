<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { clock, dur, ms, pct, rate } from "../lib/format";
  import type { AgentLine, Class, ResultStatus, WaitingKind } from "../lib/types";
  import Ant from "../components/Ant.svelte";
  import Glyph from "../components/Glyph.svelte";
  import Spark from "../components/Spark.svelte";
  import Tool from "../components/Tool.svelte";

  interface Chamber { id: string; name: string; class: Class; state: string; waiting: WaitingKind | null; for_secs: number | null; tool: string; target: string; source: string; cost: string; agents: AgentLine[] }
  interface Gallery { key: string; name: string; waiting: number; working: number; chambers: Chamber[] }
  interface Feed { session: string; session_name: string; agent: string; agent_name: string; call: string; at: string | null; tool: string; target: string; status: ResultStatus; elapsed: number | null; waiting: boolean }
  interface Row { session: string; agent: string | null; depth: number; class: Class; pid: number | null; name: string; project: string; branch: string; source: string; state: string; for_secs: number | null; tool: string; target: string; calls: number; tok_per_min: number | null; cost: string; ctx: number | null; spark: number[]; muted: boolean }
  interface Body { rows: Row[]; galleries: Gallery[]; feed: Feed[]; histogram: number[]; calls_per_min: number }

  const body = $derived(app.body<Body>("machine"));
  let hover = $state<{ g: Gallery; c: Chamber } | null>(null);
  let filter = $state("");
  let selected = $state(0);

  function open(c: Chamber) {
    app.go({ screen: "session", id: c.id });
  }

  const rows = $derived((body?.rows ?? []).filter((r) => !filter || `${r.name} ${r.project} ${r.tool} ${r.target}`.toLowerCase().includes(filter.toLowerCase())));

  export function key(e: KeyboardEvent): boolean {
    if (app.machineMode !== "table") return false;
    if (e.key === "ArrowDown" || e.key === "j") selected = Math.min(selected + 1, rows.length - 1);
    else if (e.key === "ArrowUp" || e.key === "k") selected = Math.max(selected - 1, 0);
    else if (e.key === "Enter" && rows[selected]) {
      const r = rows[selected]!;
      app.go(r.agent ? { screen: "agent", session: r.session, agent: r.agent } : { screen: "session", id: r.session });
    } else return false;
    return true;
  }

  export function selectedSession(): string | null {
    if (app.machineMode === "table") return rows[selected]?.session ?? null;
    return hover?.c.id ?? body?.galleries[0]?.chambers[0]?.id ?? null;
  }
</script>

{#if body}
  {#if app.machineMode === "map"}
    <div class="machine">
      <section class="map" aria-label="Machine map">
        <div class="caption dim">Grouped by project · depth = spawn depth</div>
        <div class="galleries">
          {#each body.galleries as g (g.key)}
            <div class="gallery">
              <button class="ghead" onclick={() => app.go({ screen: "project", key: g.key })}>
                <b>{g.name}</b> <span class="dim">· {g.chambers.length} sessions · {g.chambers.reduce((n, c) => n + c.agents.length + 1, 0)} agents</span>
                <div class="gstate">
                  {#if g.waiting}<span class="c-Waiting">{g.waiting} WAITING</span>{:else if g.working}<span class="c-Working">{g.working} working</span>{:else}<span class="c-Finished">quiet</span>{/if}
                </div>
              </button>
              <div class="shaft" class:hot={g.waiting > 0}></div>
              <div class="chambers">
                {#each g.chambers as c (c.id)}
                  <div class="colony">
                    <button class="chamber s-{c.class}" onclick={() => open(c)} onmouseenter={() => (hover = { g, c })} onmouseleave={() => (hover = null)}>
                      {#if c.waiting}<span class="badge">{c.waiting === "question" ? "?" : "!"}</span>{/if}
                      <div class="cname">{c.name}</div>
                      <div class="cstate c-{c.class}">
                        {#if c.waiting}WAITING {dur(c.for_secs)}{:else}{c.tool || c.state} · {dur(c.for_secs)}{/if}
                      </div>
                      <Ant c={c.class} />
                    </button>
                    {#if c.agents.length}
                      <svg class="tunnels" viewBox="0 0 100 20" preserveAspectRatio="none" aria-hidden="true">
                        <path d="M50 0 V10 M10 10 H90 {c.agents.map((_, i) => `M${10 + (80 * i) / Math.max(1, c.agents.length - 1)} 10 V20`).join(' ')}" class="tunnel t-{c.class}" />
                      </svg>
                      <div class="subs">
                        {#each c.agents as a (a.id)}
                          <button class="sub s-{a.class}" style="margin-left:{Math.max(0, a.depth - 1) * 14}px" onclick={() => app.go({ screen: "agent", session: c.id, agent: a.id })}>
                            <span class="sname">{a.name}</span>
                            <Ant c={a.class} size={14} />
                          </button>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/each}
              </div>
            </div>
          {/each}
        </div>
        {#if hover}
          <div class="card panel">
            <div class="dim">{hover.g.name} ›</div>
            <div><b>{hover.c.name}</b> <span class="dim">{hover.c.source}</span></div>
            <div><Tool name={hover.c.tool} /> {hover.c.target}</div>
            <div class="dim">{hover.c.state} {dur(hover.c.for_secs)} · {hover.c.agents.length} sub-agents · {hover.c.cost}</div>
            <div class="dim"><kbd>⏎</kbd> open <kbd>G</kbd> terminal</div>
          </div>
        {/if}
        <div class="legend panel">
          <span class="c-Waiting">◆ waiting for you</span>
          <span class="c-Working">● working (moves)</span>
          <span class="c-Finished">✓ finished</span>
          <span class="c-Still">○ still</span>
          <span class="dim">│ tunnel = spawned by</span>
        </div>
        <div class="minimap panel" aria-label="Minimap">
          {#each body.galleries as g (g.key)}
            <div class="mcol">
              {#each g.chambers as c (c.id)}
                <span class="mdot s-{c.class}"></span>
                {#each c.agents as a (a.id)}<span class="mdot small s-{a.class}"></span>{/each}
              {/each}
            </div>
          {/each}
        </div>
      </section>
      <aside class="feed" aria-label="Live tool calls">
        <div class="fhead"><span class="label"><span class="c-Working">●</span> live calls</span><span class="dim">{rate(body.calls_per_min)} / min</span></div>
        <div class="flist scroll">
          {#each body.feed as f (f.call)}
            <button class="fitem" class:wait={f.waiting} onclick={() => app.go({ screen: "call", session: f.session, agent: f.agent, call: f.call })}>
              <span class="dim">{clock(f.at)}</span>
              <b class:c-Waiting={f.waiting}>{f.agent_name}</b>
              <span class="fdur" class:c-Waiting={f.waiting}>{f.waiting ? "asked" : f.status === "pending" ? "…" : f.status === "error" ? "✕ " + ms(f.elapsed) : ms(f.elapsed)}</span>
              <span class="ftool"><Tool name={f.tool} /> <span class="dim">{f.target}</span></span>
            </button>
          {/each}
        </div>
        <div class="hist">
          <div class="label">calls / 10 min</div>
          <div class="bars">
            {#each body.histogram as h}
              <span class="bar" style="height:{(100 * h) / Math.max(1, ...body.histogram)}%"></span>
            {/each}
          </div>
        </div>
      </aside>
    </div>
  {:else}
    <div class="table-wrap">
      <div class="meters">
        <div><span class="c-Waiting label">waiting</span> <b class="c-Waiting">{app.view?.header.meters.waiting}</b> {#if app.view?.header.meters.oldest_wait}<span class="dim">· oldest {dur(app.view.header.meters.oldest_wait)}</span>{/if}</div>
        <div><span class="c-Working label">working</span> {app.view?.header.meters.working} agents</div>
        <div><span class="label">calls/min</span> {rate(body.calls_per_min)}</div>
        <div><span class="label">tok/min</span> {rate(app.view?.header.meters.tok_per_min ?? 0)}</div>
        <div><span class="c-Finished label">finished</span> {app.view?.header.meters.finished} agents</div>
        <div><span class="label">cost, live sessions</span> ${app.view?.header.meters.spent_today.toFixed(2)}</div>
      </div>
      <div class="controls">
        <span class="label">filter</span>
        <input id="filter" bind:value={filter} placeholder="name, project, tool…" />
        <span class="dim">{rows.length} rows</span>
      </div>
      <div class="scroll tbl">
        <table>
          <thead><tr><th></th><th>PID</th><th>AGENT</th><th>PROJECT</th><th>SOURCE</th><th>STATE</th><th>FOR</th><th>CURRENT / LAST CALL</th><th class="r">CALLS</th><th class="r">TOK/M</th><th class="r">COST</th><th class="r">CTX</th><th>ACTIVITY 10M</th></tr></thead>
          <tbody>
            {#each rows as r, i (r.session + (r.agent ?? ""))}
              <tr class:sel={i === selected} class:wait={r.class === "Waiting"} class:muted={r.muted} onclick={() => { selected = i; app.go(r.agent ? { screen: "agent", session: r.session, agent: r.agent } : { screen: "session", id: r.session }); }}>
                <td><Glyph c={r.class} pulse /></td>
                <td class="dim">{r.pid ?? ""}</td>
                <td class:root={!r.agent}>{#if r.depth}<span class="dim" style="padding-left:{(r.depth - 1) * 14}px">└ </span>{/if}{r.name}</td>
                <td>{r.project}</td>
                <td class="dim">{r.source}</td>
                <td class="c-{r.class}">{r.state}</td>
                <td class="c-{r.class}">{dur(r.for_secs)}</td>
                <td class="call"><Tool name={r.tool} /> {r.target}</td>
                <td class="r">{r.calls}</td>
                <td class="r">{r.tok_per_min ? rate(r.tok_per_min) : "-"}</td>
                <td class="r">{r.cost}</td>
                <td class="r">{pct(r.ctx)}</td>
                <td><Spark values={r.spark} c={r.class} /></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>
  {/if}
{:else}
  <p class="dim pad">Reading sessions…</p>
{/if}

<style>
  .machine {
    display: grid;
    grid-template-columns: 1fr 380px;
    height: 100%;
    min-height: 0;
  }
  .map {
    position: relative;
    overflow: auto;
    padding: 12px 16px 80px;
    background-image: radial-gradient(var(--line) 0.6px, transparent 0.6px);
    background-size: 14px 14px;
  }
  .caption {
    margin-bottom: 10px;
  }
  .galleries {
    display: flex;
    gap: 28px;
    align-items: flex-start;
  }
  .gallery {
    min-width: 160px;
  }
  .ghead {
    border: 0;
    background: none;
    padding: 0;
    text-align: left;
  }
  .gstate {
    font-size: 11px;
    font-weight: 700;
  }
  .shaft {
    height: 34px;
    border-left: 2px dashed var(--working);
    margin-left: 60px;
  }
  .shaft.hot {
    border-left: 3px solid var(--waiting);
  }
  .chambers {
    display: flex;
    gap: 16px;
    align-items: flex-start;
  }
  .colony {
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .chamber {
    position: relative;
    min-width: 140px;
    border-radius: 22px;
    padding: 10px 14px 8px;
    text-align: left;
    border: 2px solid var(--line);
    background: var(--panel);
  }
  .s-Working {
    border-color: var(--working);
    background: var(--working-bg);
  }
  .s-Waiting {
    border: 3px double var(--waiting);
    background: var(--waiting-bg);
    box-shadow: 0 0 0 4px var(--waiting-bg);
  }
  .s-Finished,
  .s-Idle {
    border-color: var(--finished);
    opacity: 0.75;
  }
  .s-Still,
  .s-Exited {
    border-style: dashed;
    border-color: var(--still);
    opacity: 0.7;
  }
  .badge {
    position: absolute;
    top: -8px;
    right: -8px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--waiting);
    color: #fff;
    font-weight: 700;
    display: grid;
    place-items: center;
  }
  .cname {
    font-weight: 700;
    max-width: 150px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cstate {
    font-size: 11px;
  }
  .tunnels {
    width: 100%;
    height: 18px;
  }
  .tunnel {
    fill: none;
    stroke: var(--line);
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }
  .t-Working {
    stroke: var(--working);
    stroke-dasharray: 4 3;
  }
  .t-Waiting {
    stroke: var(--waiting);
  }
  .subs {
    display: grid;
    grid-template-columns: repeat(2, auto);
    gap: 8px;
  }
  .sub {
    border-radius: 14px;
    padding: 4px 10px;
    min-width: 110px;
    border: 1.5px solid var(--line);
    background: var(--panel);
    text-align: left;
  }
  .sname {
    display: block;
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
  }
  .card {
    position: sticky;
    bottom: 60px;
    left: 0;
    width: 340px;
    padding: 10px 14px;
    margin-top: 16px;
  }
  .legend {
    position: absolute;
    left: 16px;
    bottom: 12px;
    display: flex;
    gap: 16px;
    padding: 6px 12px;
  }
  .minimap {
    position: absolute;
    right: 12px;
    bottom: 12px;
    display: flex;
    gap: 8px;
    padding: 8px;
  }
  .mcol {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .mdot {
    width: 12px;
    height: 7px;
    border-radius: 2px;
    border: 0;
    background: var(--line);
  }
  .mdot.small {
    width: 8px;
    margin-left: 4px;
  }
  .mdot.s-Working {
    background: var(--working);
  }
  .mdot.s-Waiting {
    background: var(--waiting);
  }
  .mdot.s-Finished,
  .mdot.s-Idle {
    background: var(--finished);
  }
  .feed {
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--line);
    min-height: 0;
  }
  .fhead {
    display: flex;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid var(--line);
  }
  .flist {
    flex: 1;
    min-height: 0;
  }
  .fitem {
    display: grid;
    grid-template-columns: 62px 1fr auto;
    gap: 2px 8px;
    width: 100%;
    text-align: left;
    border: 0;
    border-bottom: 1px solid var(--line);
    border-radius: 0;
    background: none;
    padding: 6px 14px;
  }
  .fitem.wait {
    background: var(--waiting-bg);
  }
  .ftool {
    grid-column: 2 / 4;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fdur {
    text-align: right;
  }
  .hist {
    padding: 10px 14px;
    border-top: 1px solid var(--line);
  }
  .bars {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 56px;
    margin-top: 6px;
  }
  .bar {
    flex: 1;
    background: var(--working);
    min-height: 2px;
  }
  .table-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .meters {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px 24px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
  }
  .controls {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
  }
  .controls input {
    font: inherit;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 4px 8px;
    width: 280px;
  }
  .tbl {
    flex: 1;
    min-height: 0;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--bg);
    text-align: left;
    font-weight: 400;
    color: var(--text-3);
    padding: 6px 8px;
    border-bottom: 1px solid var(--line);
  }
  td {
    padding: 5px 8px;
    border-bottom: 1px solid var(--line);
    white-space: nowrap;
  }
  tr {
    cursor: pointer;
  }
  tr.wait {
    background: var(--waiting-bg);
  }
  tr.sel {
    outline: 1px solid var(--text-3);
  }
  tr.muted {
    opacity: 0.5;
  }
  .root {
    font-weight: 700;
  }
  .call {
    max-width: 420px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .r {
    text-align: right;
  }
  .pad {
    padding: 20px;
  }
</style>
