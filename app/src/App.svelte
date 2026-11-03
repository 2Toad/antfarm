<script lang="ts">
  import { onMount } from "svelte";
  import { onView, setRoute } from "./lib/bridge";
  import type { ViewEvent } from "./lib/types";

  let view = $state<ViewEvent | null>(null);

  onMount(() => {
    let stop: (() => void) | undefined;
    onView((e) => (view = e)).then((s) => (stop = s));
    setRoute({ screen: "machine" });
    return () => stop?.();
  });
</script>

<main>
  {#if view}
    <h1>antfarm · {view.header.host}</h1>
    <p>{view.header.meters.sessions} sessions · {view.header.meters.agents} agents · {view.header.meters.waiting} waiting</p>
    <ul>
      {#each view.header.sessions as s (s.id)}
        <li>{s.name} — {s.state} — {s.tool}</li>
      {/each}
    </ul>
  {:else}
    <p>Connecting…</p>
  {/if}
</main>
