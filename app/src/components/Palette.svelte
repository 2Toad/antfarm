<script lang="ts">
  import { app } from "../lib/app.svelte";
  import Glyph from "./Glyph.svelte";

  let q = $state("");
  let index = $state(0);
  let input: HTMLInputElement | undefined = $state();

  type Item = { label: string; detail: string; run: () => void; c: import("../lib/types").Class };
  const items = $derived.by((): Item[] => {
    const h = app.view?.header;
    if (!h) return [];
    const out: Item[] = [];
    for (const s of h.sessions) {
      out.push({ label: s.name, detail: `${s.project} · ${s.state}`, c: s.class, run: () => app.go({ screen: "session", id: s.id }) });
      for (const a of s.agents) out.push({ label: a.name, detail: `${s.name} · ${a.state}`, c: a.class, run: () => app.go({ screen: "agent", session: s.id, agent: a.id }) });
    }
    for (const p of h.projects) out.push({ label: p.name, detail: `project · ${p.sessions} sessions`, c: p.waiting ? "Waiting" : "Working", run: () => app.go({ screen: "project", key: p.key }) });
    const needle = q.toLowerCase();
    return out.filter((i) => !needle || `${i.label} ${i.detail}`.toLowerCase().includes(needle)).slice(0, 12);
  });

  $effect(() => {
    if (app.palette) queueMicrotask(() => input?.focus());
  });

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") app.palette = false;
    else if (e.key === "ArrowDown") index = Math.min(index + 1, items.length - 1);
    else if (e.key === "ArrowUp") index = Math.max(index - 1, 0);
    else if (e.key === "Enter" && items[index]) {
      items[index]!.run();
      app.palette = false;
      q = "";
    } else return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

{#if app.palette}
  <div class="scrim" role="presentation" onclick={() => (app.palette = false)}></div>
  <div class="palette panel" role="dialog" aria-label="Jump">
    <input bind:this={input} bind:value={q} oninput={() => (index = 0)} onkeydown={key} placeholder="Jump to a session, agent, or project" />
    {#each items as item, i}
      <button class="item" class:on={i === index} onclick={() => { item.run(); app.palette = false; }}>
        <Glyph c={item.c} /> <b>{item.label}</b> <span class="dim">{item.detail}</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 10;
  }
  .palette {
    position: fixed;
    top: 90px;
    left: 50%;
    transform: translateX(-50%);
    width: min(560px, 90vw);
    z-index: 11;
    padding: 8px;
  }
  input {
    width: 100%;
    font: 14px var(--mono);
    padding: 8px 10px;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: 6px;
    margin-bottom: 6px;
  }
  .item {
    display: block;
    width: 100%;
    text-align: left;
    border: 0;
    background: none;
    padding: 6px 8px;
  }
  .item.on {
    background: var(--raise);
  }
</style>
