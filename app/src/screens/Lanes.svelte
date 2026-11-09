<script lang="ts">
  import { scaleTime } from "d3";
  import { app } from "../lib/app.svelte";
  import { clock, clockHM, dur, ms } from "../lib/format";
  import { CATEGORIES } from "../lib/glyph";
  import type { CallLine, Category, Class, ResultStatus, Waiting } from "../lib/types";
  import Glyph from "../components/Glyph.svelte";
  import Tool from "../components/Tool.svelte";

  interface Interval { count: number; call: string; name: string; summary: string; category: Category; status: ResultStatus; start: string; end: string; waiting: boolean }
  interface LaneI { agent: string; label: string; depth: number; class: Class; background: boolean; parent: string | null; life: [string, string] | null; spawned: string | null; calls: Interval[] }
  interface AgentAt { agent: string; label: string; depth: number; state: "NotStarted" | "Running" | "WaitingOnAgents" | "Still" | "Finished" }
  interface Body { id: string; name: string; start: string; end: string; playhead: string; following: boolean; history_start: string | null; history_cut: boolean; lanes: LaneI[]; hidden_lanes: number; agents_at: AgentAt[]; calls_at: CallLine[]; waiting: Waiting | null }

  const body = $derived(app.body<Body>("lanes"));
  const LABEL = 220;
  const ROW = 34;
  let width = $state(1000);

  const x = $derived(body ? scaleTime().domain([new Date(body.start), new Date(body.end)]).range([0, Math.max(100, width - LABEL - 16)]) : null);
  const ticks = $derived(x ? x.ticks(Math.max(3, Math.floor((width - LABEL) / 120))) : []);
  // Ticks in the same 24-hour clock as everything else; seconds only when zoomed in.
  const tickFormat = $derived((t: Date) => (windowSecs() <= 300 ? clock(t.toISOString()) : clockHM(t.toISOString())));
  // Keeps the playhead's time label inside the chart at either edge.
  const labelShift = $derived.by(() => {
    if (!body || !x) return 0;
    const px = x(new Date(body.playhead));
    return Math.min(0, x.range()[1]! - px - 36);
  });
  const rowOf = $derived(new Map((body?.lanes ?? []).map((l, i) => [l.agent, i])));

  type Span = { secs: number; label: string };
  const PRESETS: Span[] = [
    { secs: 60, label: "1m" },
    { secs: 300, label: "5m" },
    { secs: 900, label: "15m" },
    { secs: 3600, label: "1h" },
    { secs: 7200, label: "2h" },
    { secs: 21600, label: "6h" },
    { secs: 86400, label: "1d" },
  ];

  function windowSecs(): number {
    return body ? (new Date(body.end).getTime() - new Date(body.start).getTime()) / 1000 : 900;
  }

  /**
   * Moves to a new window. An `end` of null, or one at now, follows now: the
   * backend slides a window of the same span with the clock.
   */
  function setWindow(start: Date, end: Date | null, playhead: Date | null) {
    if (!body) return;
    const now = new Date(app.view!.now).getTime();
    const following = !end || end.getTime() >= now - 1000;
    const span = Math.round(((end?.getTime() ?? now) - start.getTime()) / 1000);
    app.replace({
      screen: "lanes",
      id: body.id,
      start: following ? null : start.toISOString(),
      end: following ? null : end!.toISOString(),
      playhead: playhead ? playhead.toISOString() : null,
      span: Math.max(30, Math.min(30 * 86400, span)),
      width: Math.max(100, Math.round(width - LABEL)),
    });
  }

  function zoom(factor: number, around?: Date) {
    if (!body) return;
    const secs = Math.max(30, Math.min(30 * 86400, windowSecs() * factor));
    const now = new Date(app.view!.now).getTime();
    const centre = (around ?? new Date(body.playhead)).getTime();
    const end = Math.min(now, centre + (secs * 1000) / 2);
    setWindow(new Date(end - secs * 1000), new Date(end), body.following ? null : new Date(body.playhead));
  }

  function preset(p: Span) {
    if (!body) return;
    const end = body.following ? new Date(app.view!.now) : new Date(body.end);
    setWindow(new Date(end.getTime() - p.secs * 1000), body.following ? null : end, null);
  }

  function fit() {
    if (!body?.history_start) return;
    setWindow(new Date(body.history_start), null, null);
  }

  function now() {
    if (!body) return;
    const secs = windowSecs();
    const n = new Date(app.view!.now);
    setWindow(new Date(n.getTime() - secs * 1000), null, null);
  }

  function seek(t: Date) {
    if (!body) return;
    setWindow(new Date(body.start), new Date(body.end), t);
  }

  function stepCall(forward: boolean) {
    if (!body) return;
    const p = new Date(body.playhead).getTime();
    let next = forward ? Infinity : -Infinity;
    for (const l of body.lanes) {
      for (const c of l.calls) {
        const s = new Date(c.start).getTime();
        if (forward ? s > p && s < next : s < p && s > next) next = s;
      }
    }
    if (Number.isFinite(next)) seek(new Date(next));
  }

  export function key(e: KeyboardEvent): boolean {
    if (!body || !x) return false;
    const colMs = (new Date(body.end).getTime() - new Date(body.start).getTime()) / Math.max(1, width - LABEL);
    const p = new Date(body.playhead).getTime();
    switch (e.key) {
      case "ArrowLeft": seek(new Date(p - colMs * 8)); break;
      case "ArrowRight": seek(new Date(p + colMs * 8)); break;
      case ",": stepCall(false); break;
      case ".": stepCall(true); break;
      case "+": case "=": zoom(0.5); break;
      case "-": case "_": zoom(2); break;
      case "f": case "F": fit(); break;
      case "End": case "0": now(); break;
      default: return false;
    }
    return true;
  }

  let svg: SVGSVGElement | undefined = $state();
  let dragging = false;
  function timeAt(ev: PointerEvent | WheelEvent): Date | null {
    if (!svg || !x) return null;
    const r = svg.getBoundingClientRect();
    return x.invert(Math.max(0, ev.clientX - r.left - LABEL));
  }
  function down(ev: PointerEvent) {
    dragging = true;
    (ev.currentTarget as Element).setPointerCapture(ev.pointerId);
    const t = timeAt(ev);
    if (t) seek(t);
  }
  function move(ev: PointerEvent) {
    if (!dragging) return;
    const t = timeAt(ev);
    if (t) seek(t);
  }
  function wheel(ev: WheelEvent) {
    if (!ev.ctrlKey && Math.abs(ev.deltaY) < Math.abs(ev.deltaX)) return;
    ev.preventDefault();
    zoom(ev.deltaY > 0 ? 1.25 : 0.8, timeAt(ev) ?? undefined);
  }

  const stateClass = (s: AgentAt["state"]): Class => (s === "Finished" ? "Finished" : s === "Still" ? "Still" : "Working");
  const stateLabel = (s: AgentAt["state"]) => ({ NotStarted: "not started", Running: "working", WaitingOnAgents: "waiting on agents", Still: "still", Finished: "finished" })[s];
  const doneBy = (c: CallLine, p: string) => c.at != null && c.elapsed != null && new Date(c.at).getTime() + c.elapsed <= new Date(p).getTime();
</script>

{#if body && x}
  <div class="lanes">
    {#if body.waiting}
      <div class="wline"><b class="c-Waiting">THIS SESSION IS WAITING</b> {body.waiting.kind} · {body.waiting.request} <b class="c-Waiting">{dur(body.waiting.age)}</b><span class="spacer"></span><kbd>G</kbd> go to terminal</div>
    {/if}
    <div class="controls">
      <span class="label">time</span>
      <button onclick={() => zoom(2)} title="Zoom out">−</button>
      <span class="seg">
        {#each PRESETS as p}<button class:on={Math.abs(windowSecs() - p.secs) < 2} onclick={() => preset(p)}>{p.label}</button>{/each}
      </span>
      <button onclick={() => zoom(0.5)} title="Zoom in">+</button>
      <button onclick={fit}>Fit <kbd>F</kbd></button>
      <span class="dim">{clock(body.start)} → {clock(body.end)}</span>
      <button class="nowbtn" class:on={body.following} onclick={now}><span class="c-Waiting">◆</span> Jump to now</button>
      <span class="spacer"></span>
      {#each CATEGORIES as c}<span class="key"><span class="sw cat-{c}"></span>{c === "ask" ? "ask / permission" : c}</span>{/each}
      <span class="key"><span class="sw hatch"></span>failed</span>
    </div>
    <div class="chart" data-tour="lanes" bind:clientWidth={width}>
      <svg bind:this={svg} width={width} height={28 + body.lanes.length * ROW + 8} onpointerdown={down} onpointermove={move} onpointerup={() => (dragging = false)} onwheel={wheel} role="img" aria-label="Swimlanes">
        <defs>
          <pattern id="hatch" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
            <rect width="5" height="5" fill="var(--error)" opacity="0.35" />
            <line x1="0" y1="0" x2="0" y2="5" stroke="var(--error)" stroke-width="2.5" />
          </pattern>
          <marker id="arrow" viewBox="0 0 8 8" refX="4" refY="7" markerWidth="7" markerHeight="7" orient="auto">
            <path d="M0 0 L8 0 L4 8 z" fill="var(--text-2)" />
          </marker>
        </defs>
        <text class="axislabel" x="12" y="18">AGENT · BY SPAWN DEPTH</text>
        {#each ticks as t}
          <g transform="translate({LABEL + x(t)},0)">
            <line y1="24" y2={28 + body.lanes.length * ROW} class="grid" />
            <text y="18" class="tick" text-anchor="middle">{tickFormat(t)}</text>
          </g>
        {/each}
        {#each body.lanes as l, i (l.agent)}
          <g transform="translate(0,{28 + i * ROW})">
            <rect class="row" width={width} height={ROW} />
            <foreignObject x="0" y="0" width={LABEL - 8} height={ROW}>
              <button class="lname" style="padding-left:{8 + l.depth * 14}px" onclick={() => app.go({ screen: "agent", session: body.id, agent: l.agent })}>
                {l.depth ? "└ " : ""}<Glyph c={l.class} /> {l.label}
              </button>
            </foreignObject>
            {#if l.life}
              <line x1={LABEL + Math.max(0, x(new Date(l.life[0])))} x2={LABEL + Math.min(x.range()[1]!, x(new Date(l.life[1])))} y1={ROW / 2} y2={ROW / 2} class="life" />
            {/if}
            {#each l.calls as c (c.call)}
              {@const a = Math.max(0, x(new Date(c.start)))}
              {@const b = Math.min(x.range()[1]!, x(new Date(c.end)))}
              <rect x={LABEL + a} y={ROW / 2 - 7} width={Math.max(3, b - a)} height="14" rx="2" class="bar cat-{c.category}" class:running={c.status === "pending"} class:waiting={c.waiting} fill={c.status === "error" ? "url(#hatch)" : null} onpointerdown={(e) => e.stopPropagation()} onclick={() => app.go({ screen: "call", session: body.id, agent: l.agent, call: c.call })} onkeydown={(e) => e.key === "Enter" && app.go({ screen: "call", session: body.id, agent: l.agent, call: c.call })} role="button" tabindex="-1">
                <title>{c.count > 1 ? c.summary : `${c.name} · ${c.summary}`} · {clock(c.start)} · {c.status}</title>
              </rect>
            {/each}
          </g>
        {/each}
        {#each body.lanes as l (l.agent)}
          {#if l.parent && l.spawned && rowOf.has(l.parent) && x(new Date(l.spawned)) >= 0}
            {@const from = rowOf.get(l.parent)!}
            {@const to = rowOf.get(l.agent)!}
            <line class="spawn" x1={LABEL + x(new Date(l.spawned))} x2={LABEL + x(new Date(l.spawned))} y1={28 + from * ROW + ROW / 2 + 6} y2={28 + to * ROW + ROW / 2 - 8} marker-end="url(#arrow)" />
          {/if}
        {/each}
        <g transform="translate({LABEL + x(new Date(body.playhead))},0)" class="playhead">
          <line y1="22" y2={28 + body.lanes.length * ROW} />
          <rect x={-34 + labelShift} y="4" width="68" height="18" rx="3" />
          <text x={labelShift} y="17" text-anchor="middle">{clock(body.playhead)}</text>
        </g>
      </svg>
    </div>
    <div class="scrub" data-tour="lanes">
      <span class="label">playhead</span>
      <input type="range" min={new Date(body.start).getTime()} max={new Date(body.end).getTime()} value={new Date(body.playhead).getTime()} oninput={(e) => seek(new Date(Number(e.currentTarget.value)))} />
      <span class="dim">drag on the lanes or here · <kbd>,</kbd> <kbd>.</kbd> step one call</span>
      {#if body.history_cut}<span class="dim">· older calls not kept</span>{/if}
      {#if body.hidden_lanes}<span class="dim">· {body.hidden_lanes} more agents not drawn</span>{/if}
    </div>
    <div class="panels">
      <section>
        <div class="label pad">agents at {clock(body.playhead)}</div>
        {#each body.agents_at as a (a.agent)}
          <div class="arow" style="padding-left:{16 + a.depth * 16}px"><Glyph c={stateClass(a.state)} /> <b>{a.label}</b><span class="spacer"></span><span class="c-{stateClass(a.state)}">{stateLabel(a.state)}</span></div>
        {/each}
      </section>
      <section class="calls">
        <div class="label pad">calls up to {clock(body.playhead)} · newest first</div>
        {#each body.calls_at as c (c.agent + c.call)}
          <button class="crow" onclick={() => app.go({ screen: "call", session: body.id, agent: c.agent, call: c.call })}>
            <span class="dim">{clock(c.at)}</span>
            <span class="who">{c.agent_name}</span>
            <span class="sw cat-{c.tool ? '' : 'other'}"></span>
            <span><Tool name={c.tool} /> {c.target}</span>
            <span class="res">{doneBy(c, body.playhead) ? (c.status === "error" ? "✕ " : "") + ms(c.elapsed) : "running"}</span>
          </button>
        {/each}
      </section>
    </div>
  </div>
{:else}
  <p class="dim" style="padding:20px">{app.gone ? "This session is gone." : "Loading…"}</p>
{/if}

<style>
  .lanes {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .wline {
    display: flex;
    gap: 12px;
    padding: 6px 16px;
    background: var(--waiting-bg);
    border-bottom: 2px solid var(--waiting);
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    flex-wrap: wrap;
  }
  .seg {
    display: inline-flex;
  }
  .seg button {
    border-radius: 0;
    margin-left: -1px;
  }
  .seg .on,
  .nowbtn.on {
    background: var(--text);
    color: var(--bg);
  }
  .spacer {
    flex: 1;
  }
  .key {
    display: inline-flex;
    gap: 5px;
    align-items: center;
    color: var(--text-2);
  }
  .sw {
    display: inline-block;
    width: 14px;
    height: 10px;
    border-radius: 2px;
  }
  .chart {
    overflow: auto;
    flex: none;
    max-height: 45vh;
    border-bottom: 1px solid var(--line);
  }
  svg {
    display: block;
    cursor: crosshair;
    user-select: none;
  }
  .axislabel,
  .tick {
    fill: var(--text-3);
    font: 11px var(--mono);
  }
  .grid {
    stroke: var(--line);
    stroke-dasharray: 2 4;
  }
  .row {
    fill: transparent;
    stroke: var(--line);
    stroke-width: 0.5;
  }
  .lname {
    width: 100%;
    height: 100%;
    border: 0;
    background: none;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    padding: 0;
  }
  .life {
    stroke: var(--text-3);
    stroke-dasharray: 2 3;
  }
  .bar {
    cursor: pointer;
  }
  .bar.running {
    opacity: 0.6;
  }
  .bar.waiting {
    fill: var(--waiting);
  }
  .cat-shell { fill: var(--cat-shell); background: var(--cat-shell); }
  .cat-read { fill: var(--cat-read); background: var(--cat-read); }
  .cat-edit { fill: var(--cat-edit); background: var(--cat-edit); }
  .cat-search { fill: var(--cat-search); background: var(--cat-search); }
  .cat-web { fill: var(--cat-web); background: var(--cat-web); }
  .cat-spawn { fill: transparent; stroke: var(--text-2); stroke-width: 1.5; background: transparent; border: 1.5px solid var(--text-2); }
  .cat-ask { fill: var(--cat-ask); background: var(--cat-ask); }
  .cat-other { fill: var(--cat-other); background: var(--cat-other); }
  .hatch {
    background: repeating-linear-gradient(45deg, var(--error) 0 3px, transparent 3px 6px);
  }
  .spawn {
    stroke: var(--text-2);
    stroke-width: 1.2;
  }
  .playhead line {
    stroke: var(--text);
    stroke-width: 1.5;
  }
  .playhead rect {
    fill: var(--text);
  }
  .playhead text {
    fill: var(--bg);
    font: 700 11px var(--mono);
  }
  .scrub {
    display: flex;
    gap: 14px;
    align-items: center;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
  }
  .scrub input {
    flex: 1;
    max-width: 640px;
    accent-color: var(--text);
  }
  .panels {
    display: grid;
    grid-template-columns: 2fr 3fr;
    flex: 1;
    min-height: 0;
  }
  .panels section {
    overflow: auto;
  }
  .calls {
    border-left: 1px solid var(--line);
  }
  .pad {
    padding: 10px 16px;
  }
  .arow {
    display: flex;
    gap: 8px;
    padding: 4px 16px;
    border-bottom: 1px solid var(--line);
  }
  .crow {
    display: grid;
    grid-template-columns: 70px 150px 14px 1fr 80px;
    gap: 10px;
    align-items: center;
    width: 100%;
    text-align: left;
    border: 0;
    border-bottom: 1px solid var(--line);
    border-radius: 0;
    background: none;
    padding: 5px 16px;
    white-space: nowrap;
  }
  .crow span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .res {
    text-align: right;
  }
</style>
