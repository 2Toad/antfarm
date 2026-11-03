<script lang="ts">
  import { app } from "../lib/app.svelte";
  import Ant from "./Ant.svelte";

  const crumbs = $derived.by(() => {
    const h = app.view?.header;
    const out: { label: string; reached: boolean; depth: number }[] = [{ label: h?.host ?? "machine", reached: true, depth: 0 }];
    const r = app.route;
    const session = "id" in r ? r.id : "session" in r ? r.session : null;
    const s = session ? h?.sessions.find((x) => x.id === session) : null;
    const project = r.screen === "project" ? h?.projects.find((p) => p.key === r.key) : s ? { name: s.project, key: s.project_key } : null;
    out.push({ label: project ? project.name : "project", reached: !!project, depth: 1 });
    out.push({ label: s ? s.name : "session", reached: !!s, depth: 2 });
    if (r.screen === "lanes") out.push({ label: "swimlanes", reached: true, depth: 3 });
    else out.push({ label: r.screen === "agent" || r.screen === "call" ? agentName() : "agent", reached: r.screen === "agent" || r.screen === "call", depth: 3 });
    if (r.screen !== "lanes") out.push({ label: "call", reached: r.screen === "call", depth: 4 });
    return out;
  });

  function agentName(): string {
    const b = app.view?.body as { agent_name?: string; name?: string } | undefined;
    return b?.agent_name ?? b?.name ?? "agent";
  }

  const level = $derived(crumbs.filter((c) => c.reached).length);

  function jump(depth: number) {
    while (app.stack.length > 1 && app.stack.length - 1 > depth) app.back();
    if (depth === 0) app.home();
  }

  const m = $derived(app.view?.header.meters);
</script>

<header class="bar" data-tauri-drag-region>
  <span class="brand" data-tauri-drag-region><Ant c="Working" size={20} /> antfarm</span>
  <nav aria-label="Zoom depth">
    {#each crumbs as c, i}
      {#if i > 0}<span class="sep">›</span>{/if}
      <button class="crumb" class:reached={c.reached} disabled={!c.reached} onclick={() => jump(c.depth)}>{c.label}</button>
    {/each}
    <kbd>{level}/5</kbd>
  </nav>
  <button class="jump" onclick={() => (app.palette = true)}>Jump to session or agent <kbd>⌘K</kbd></button>
  {#if m}
    <span class="pill waiting" class:none={m.waiting === 0}>{m.waiting} WAITING</span>
    <span class="c-Working">● {m.working} working</span>
    <span class="c-Finished">✓ {m.finished} done</span>
    <span class="c-Still">○ {m.still} still</span>
  {/if}
  {#if app.route.screen === "machine"}
    <span class="seg">
      <button class:on={app.machineMode === "map"} onclick={() => (app.machineMode = "map")}>Map</button>
      <button class:on={app.machineMode === "table"} onclick={() => (app.machineMode = "table")}>Table</button>
    </span>
  {/if}
  <button class="theme" title="Theme" onclick={() => app.setTheme(app.theme === "light" ? "dark" : "light")}>{app.theme === "light" ? "☾" : "☀"}</button>
</header>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 44px;
    padding: 0 14px 0 84px;
    border-bottom: 1px solid var(--line);
    background: var(--bg);
    -webkit-app-region: drag;
    white-space: nowrap;
  }
  .bar > :global(*) {
    -webkit-app-region: no-drag;
  }
  .brand {
    font-weight: 700;
    font-size: 14px;
    -webkit-app-region: drag;
  }
  nav {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
  .crumb {
    border: 0;
    background: none;
    padding: 0;
    color: var(--text-3);
  }
  .crumb.reached {
    color: var(--text);
    cursor: pointer;
  }
  .crumb:disabled {
    cursor: default;
  }
  .sep {
    color: var(--text-3);
  }
  .jump {
    color: var(--text-3);
    min-width: 240px;
    text-align: left;
  }
  .pill.waiting {
    background: var(--waiting);
    color: #fff;
    font-weight: 700;
    padding: 3px 10px;
    border-radius: 5px;
  }
  .pill.waiting.none {
    background: var(--raise);
    color: var(--text-3);
  }
  .seg {
    display: inline-flex;
  }
  .seg button {
    border-radius: 0;
  }
  .seg button:first-child {
    border-radius: 6px 0 0 6px;
  }
  .seg button:last-child {
    border-radius: 0 6px 6px 0;
    border-left: 0;
  }
  .seg .on {
    background: var(--text);
    color: var(--bg);
  }
</style>
