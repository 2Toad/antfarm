<script lang="ts">
  let { value, width = 90 }: { value: number | null | undefined; width?: number } = $props();
  const level = $derived(value == null ? "none" : value >= 80 ? "hot" : value >= 60 ? "warm" : "ok");
</script>

{#if value == null}
  <span class="dim">-</span>
{:else}
  <span class="gauge" style="width:{width}px" title="{Math.round(value)}% of the context window">
    <span class="fill {level}" style="width:{Math.min(100, value)}%"></span>
  </span>
  <span class="n">{Math.round(value)}%</span>
{/if}

<style>
  .gauge {
    display: inline-block;
    height: 6px;
    border-radius: 3px;
    background: var(--raise);
    border: 1px solid var(--line);
    vertical-align: middle;
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--text-2);
  }
  .fill.warm {
    background: var(--working);
  }
  .fill.hot {
    background: var(--waiting);
  }
  .n {
    margin-left: 6px;
  }
</style>
