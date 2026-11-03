<script lang="ts">
  import { glyph } from "../lib/glyph";
  import type { Class } from "../lib/types";
  let { c, pulse = false }: { c: Class; pulse?: boolean } = $props();
</script>

<span class="g c-{c}" class:pulse={pulse && c === "Working"} aria-label={c.toLowerCase()}>{glyph(c)}</span>

<style>
  .g {
    display: inline-block;
    width: 1.1em;
    text-align: center;
  }
  .pulse {
    animation: pulse 1.6s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
</style>
