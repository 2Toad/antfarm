<script lang="ts">
  import { app } from "../lib/app.svelte";
  const keys: [string, string][] = [
    ["Enter", "drill in: project, session, agent, call"],
    ["Esc", "back out"],
    ["⌘K", "jump to a session, agent, or project"],
    ["N", "next session waiting for you"],
    ["G", "go to the session's terminal or editor"],
    ["C", "copy the command it is waiting on"],
    ["Z", "snooze a waiting session for 10 minutes"],
    ["M", "mute a session"],
    ["L", "swimlanes for the session"],
    ["/", "filter the table"],
    ["S", "sort the table: needs you, cost, name"],
    ["T", "show or hide sub-agents"],
    ["f", "show or hide finished sub-agents"],
    ["a", "show or hide exited sessions"],
    ["↑ ↓", "move the selection on the map, table, and session"],
    ["⌘⇧A", "bring antfarm forward from anywhere"],
  ];
</script>

{#if app.help}
  <div class="scrim" role="presentation" onclick={() => (app.help = false)}></div>
  <div class="help panel" role="dialog" aria-label="Keys">
    <div class="label">keys · any key closes</div>
    {#each keys as [k, d]}
      <div class="row"><kbd>{k}</kbd><span>{d}</span></div>
    {/each}
    <p class="prose dim">antfarm only watches. You answer in the terminal.</p>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 10;
  }
  .help {
    position: fixed;
    top: 15%;
    left: 50%;
    transform: translateX(-50%);
    width: min(520px, 90vw);
    padding: 16px 20px;
    z-index: 11;
  }
  .row {
    display: grid;
    grid-template-columns: 90px 1fr;
    padding: 3px 0;
  }
</style>
