<script lang="ts">
  import { categoryOf } from "../lib/glyph";
  let { name }: { name: string } = $props();
</script>

<span class="tool cat-{categoryOf(name)}">{name || "—"}</span>

<style>
  .tool {
    display: inline-block;
    min-width: 3.5em;
  }
  .cat-shell { color: var(--cat-shell); }
  .cat-read { color: var(--cat-read); }
  .cat-edit { color: var(--cat-edit); }
  .cat-search { color: var(--cat-search); }
  .cat-web { color: var(--cat-web); }
  .cat-spawn { color: var(--cat-spawn); }
  .cat-ask { color: var(--cat-ask); }
  .cat-other { color: var(--text-2); }
</style>
