<script lang="ts">
  import type { Class } from "../lib/types";
  let { values, c = "Working", width = 120, height = 18 }: { values: number[]; c?: Class; width?: number; height?: number } = $props();
  const points = $derived.by(() => {
    const max = Math.max(1, ...values);
    const step = width / Math.max(1, values.length - 1);
    return values.map((v, i) => `${(i * step).toFixed(1)},${(height - 2 - (v / max) * (height - 4)).toFixed(1)}`).join(" ");
  });
</script>

<svg {width} {height} class="c-{c}" aria-hidden="true">
  <polyline points={points} fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
</svg>
