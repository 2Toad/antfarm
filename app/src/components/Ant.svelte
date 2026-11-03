<script lang="ts">
  import type { Class } from "../lib/types";
  let { c, size = 18 }: { c: Class; size?: number } = $props();
</script>

<svg class="ant c-{c}" class:walk={c === "Working"} width={size} height={size * 0.6} viewBox="0 0 30 18" aria-hidden="true">
  <g fill="currentColor" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
    <ellipse cx="7" cy="9" rx="4.5" ry="4" stroke="none" />
    <ellipse cx="15" cy="9" rx="3.4" ry="3" stroke="none" />
    <ellipse cx="22.5" cy="9" rx="5" ry="4.2" stroke="none" />
    <path d="M15 9 L11 2 M15 9 L15 1.5 M15 9 L19 2 M15 9 L11 16 M15 9 L15 16.5 M15 9 L19 16" fill="none" />
    <path d="M3 7 L0.5 3.5 M3 11 L0.5 14.5" fill="none" />
  </g>
</svg>

<style>
  .ant {
    display: inline-block;
  }
  .walk {
    animation: walk 1.8s ease-in-out infinite alternate;
  }
  @keyframes walk {
    from {
      transform: translateX(0);
    }
    to {
      transform: translateX(14px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .walk {
      animation: none;
    }
  }
</style>
