<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { clock, dur, ms, pct } from "../lib/format";
  import type { Class, ResultStatus } from "../lib/types";
  import Glyph from "../components/Glyph.svelte";
  import Tool from "../components/Tool.svelte";

  interface Water { first: number; count: number; tool: string; target: string; at: string | null; elapsed_ms: number | null; status: ResultStatus; call: string }
  interface Body { session: string; session_name: string; id: string; name: string; kind: string; class: Class; state: string; depth: number; read: boolean; background: boolean; model: string | null; effort: string | null; cost: number | null; context_pct: number | null; elapsed_secs: number | null; brief: string | null; parent: string | null; calls: Water[] }

  const body = $derived(app.body<Body>("agent"));
  let selected = $state(-1);

  const span = $derived.by(() => {
    const ts = (body?.calls ?? []).flatMap((c) => (c.at ? [new Date(c.at).getTime()] : []));
    if (!ts.length) return null;
    const a = Math.min(...ts);
    const now = new Date(app.view!.now).getTime();
    return { a, b: Math.max(now, a + 1000) };
  });
  const pos = (c: Water) => {
    if (!span || !c.at) return { left: 0, width: 0 };
    const t = new Date(c.at).getTime();
    const end = c.elapsed_ms != null ? t + c.elapsed_ms : new Date(app.view!.now).getTime();
    const w = span.b - span.a;
    return { left: ((t - span.a) / w) * 100, width: Math.max(0.6, ((end - t) / w) * 100) };
  };

  export function key(e: KeyboardEvent): boolean {
    if (!body) return false;
    if (e.key === "ArrowDown" || e.key === "j") selected = Math.min(selected + 1, body.calls.length - 1);
    else if (e.key === "ArrowUp" || e.key === "k") selected = Math.max(selected - 1, 0);
    else if (e.key === "Enter" && body.calls[selected]) app.go({ screen: "call", session: body.session, agent: body.id, call: body.calls[selected]!.call });
    else return false;
    return true;
  }
</script>

{#if body}
  <div class="agent">
    <div class="head">
      <div class="label">agent · depth {body.depth}</div>
      <div class="title">
        <h1>{body.name}</h1>
        <span class="c-{body.class}"><Glyph c={body.class} pulse /> {body.state} · {dur(body.elapsed_secs)}</span>
        <span>{body.kind} · {body.background ? "background" : "foreground"}</span>
        <span>{body.model ?? "-"}{body.effort ? ` · ${body.effort}` : ""}</span>
        <span>{body.cost == null ? "-" : `$${body.cost.toFixed(2)}`}</span>
        <span>context {pct(body.context_pct)}</span>
      </div>
      {#if body.brief}<p class="prose brief">{body.parent ? `Spawned by ${body.parent}: ` : ""}“{body.brief}”</p>{/if}
    </div>
    <div class="water scroll">
      {#if !body.read}<p class="dim loading">Loading this agent's history…</p>{/if}
      <div class="wrow whead"><span>#</span><span>TOOL</span><span>TARGET</span><span class="dim">{span ? clock(new Date(span.a).toISOString()) : ""}</span><span class="r">RESULT</span></div>
      {#each body.calls as c, i (c.call)}
        <button class="wrow" class:sel={i === selected} class:run={c.status === "pending"} onclick={() => app.go({ screen: "call", session: body.session, agent: body.id, call: c.call })}>
          <span class="dim">{c.count > 1 ? `${c.first}–${c.first + c.count - 1}` : c.first}</span>
          <span><Tool name={c.tool} />{c.count > 1 ? ` ×${c.count}` : ""}</span>
          <span class="target">{c.target}</span>
          <span class="track"><span class="bar s-{c.status}" style="left:{pos(c).left}%;width:{pos(c).width}%"></span></span>
          <span class="r" class:err={c.status === "error"}>{c.status === "error" ? "✕ " : c.status === "ok" ? "ok " : ""}{c.status === "pending" ? "running" : ms(c.elapsed_ms)}</span>
        </button>
      {/each}
      <div class="legend dim"><span><span class="sw s-ok"></span> exit ok</span><span><span class="sw s-error"></span> ✕ exit error (a shape, not a color)</span><span><span class="sw s-pending"></span> running</span></div>
    </div>
  </div>
{:else}
  <p class="dim" style="padding:20px">{app.gone ? "This agent is gone." : "Loading…"}</p>
{/if}

<style>
  .loading { padding: 8px 0; }
  .agent {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .head {
    padding: 16px 20px;
    border-bottom: 1px solid var(--line);
  }
  .title {
    display: flex;
    flex-wrap: wrap;
    gap: 18px;
    align-items: baseline;
  }
  h1 {
    margin: 0;
    font-size: 24px;
  }
  .brief {
    margin: 8px 0 0;
    max-width: 900px;
  }
  .water {
    flex: 1;
    min-height: 0;
  }
  .wrow {
    display: grid;
    grid-template-columns: 70px 140px minmax(0, 1fr) 360px 110px;
    gap: 12px;
    width: 100%;
    text-align: left;
    align-items: center;
    border: 0;
    border-bottom: 1px solid var(--line);
    border-radius: 0;
    background: none;
    padding: 6px 20px;
  }
  .whead {
    color: var(--text-3);
    position: sticky;
    top: 0;
    background: var(--bg);
  }
  .wrow.sel,
  .wrow.run {
    outline: 1px solid var(--working);
    outline-offset: -1px;
  }
  .target {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .track {
    position: relative;
    height: 10px;
  }
  .bar {
    position: absolute;
    top: 0;
    height: 10px;
    border-radius: 2px;
  }
  .s-ok {
    background: var(--finished);
  }
  .s-error {
    background: repeating-linear-gradient(45deg, var(--error) 0 3px, transparent 3px 6px);
    border: 1px solid var(--error);
  }
  .s-pending {
    background: repeating-linear-gradient(90deg, var(--working) 0 6px, transparent 6px 9px);
  }
  .r {
    text-align: right;
  }
  .err {
    color: var(--error);
  }
  .legend {
    display: flex;
    gap: 20px;
    padding: 12px 20px;
  }
  .sw {
    display: inline-block;
    width: 18px;
    height: 8px;
    vertical-align: middle;
  }
</style>
