<script lang="ts">
  import { app } from "../lib/app.svelte";
  const keys = $derived.by((): [string, string][] => {
    switch (app.route.screen) {
      case "machine":
        return [["⌘K", "jump"], ["N", "next waiting"], ["⏎", "drill in"], ["/", "filter"], ["Esc", "back out"], ["?", "keys"]];
      case "session":
        return [["G", "go to terminal"], ["C", "copy command"], ["Z", "snooze"], ["L", "swimlanes"], ["N", "next waiting"], ["Esc", "back out"]];
      case "lanes":
        return [["←→", "playhead"], [", .", "step calls"], ["+ −", "zoom"], ["F", "fit"], ["End", "now"], ["Esc", "back out"]];
      default:
        return [["↑↓", "select"], ["⏎", "open"], ["G", "go to terminal"], ["Esc", "back out"]];
    }
  });
  const h = $derived(app.view?.header);
</script>

<footer>
  {#if app.toast}
    <span class="toast">{app.toast.text}</span>
  {:else if h}
    <span class="dim">{h.meters.sessions} sessions · {h.meters.agents} agents</span>
    {#if h.meters.apps.length}<span class="dim">watching {h.meters.apps.join(", ")}</span>{/if}
  {/if}
  <span class="spacer"></span>
  {#each keys as [k, label]}
    <span><kbd>{k}</kbd> <span class="dim">{label}</span></span>
  {/each}
</footer>

<style>
  footer {
    display: flex;
    gap: 16px;
    align-items: center;
    height: 28px;
    padding: 0 14px;
    border-top: 1px solid var(--line);
    white-space: nowrap;
    overflow: hidden;
  }
  .spacer {
    flex: 1;
  }
  .toast {
    color: var(--text);
    font-weight: 600;
  }
</style>
