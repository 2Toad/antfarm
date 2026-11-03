<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { dur } from "../lib/format";
  let { except = null }: { except?: string | null } = $props();
  const items = $derived((app.view?.header.needs_you ?? []).filter((n) => n.session !== except));
</script>

{#if items.length}
  <section class="needs" aria-label="Waiting for you">
    <div class="count"><span class="label">needs you</span><b>{items.length}</b></div>
    <div class="rows">
      {#each items.slice(0, 3) as n, i (n.session)}
        <div class="row" class:first={i === 0}>
          <span class="c-Waiting">◆</span>
          <button class="link" onclick={() => app.go({ screen: "session", id: n.session })}><span class="dim">{n.project} ›</span> <b>{n.name}</b></button>
          <span class="kind">{n.kind.toUpperCase()}</span>
          <span class="req">{n.request}</span>
          <span class="dim">{n.source}</span>
          <span class="age c-Waiting">{dur(n.age)}</span>
          <button class="go" onclick={() => app.run("go_to_terminal", { session: n.session })}>Go to terminal <kbd>G</kbd></button>
        </div>
      {/each}
    </div>
  </section>
{/if}

<style>
  .needs {
    display: flex;
    border-bottom: 2px solid var(--waiting);
    background: var(--waiting-bg);
  }
  .count {
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 6px 18px;
    background: var(--waiting);
    color: #fff;
    min-width: 120px;
  }
  .count .label {
    color: #fff;
  }
  .count b {
    font-size: 26px;
    line-height: 1;
  }
  .rows {
    flex: 1;
    min-width: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 14px;
    white-space: nowrap;
  }
  .row + .row {
    border-top: 1px solid var(--line);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
  }
  .kind {
    border: 1px solid var(--waiting);
    color: var(--waiting);
    border-radius: 4px;
    padding: 0 6px;
    font-weight: 700;
    font-size: 11px;
  }
  .req {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .age {
    font-weight: 700;
  }
  .go {
    border-color: var(--waiting);
    color: var(--waiting);
  }
  .first .go {
    background: var(--waiting);
    color: #fff;
  }
</style>
