<script lang="ts">
  import { tick } from "svelte";
  import { app } from "../lib/app.svelte";
  import { away as isAway, SCREENS } from "../lib/tour";

  const GAP = 14;
  const MARGIN = 12;
  const WIDTH = 380;

  let dontShow = $state(false);
  let card: HTMLDivElement | undefined = $state();
  let cardH = $state(0);
  /** The target's box, or null when it isn't on screen. */
  let rect = $state<DOMRect | null>(null);
  let innerW = $state(1200);
  let innerH = $state(800);

  const run = $derived(app.tour);
  const tip = $derived(run ? run.tips[run.index]! : null);
  const last = $derived(run ? run.index === run.tips.length - 1 : false);
  /** Somewhere the step didn't ask for: the card explains this screen instead. */
  const away = $derived(tip ? isAway(tip, app.route) : false);
  const note = $derived(SCREENS[app.route.screen]);

  // Once the person does what the step asks, move on, even if they got there their own way.
  $effect(() => {
    if (!tip?.done) return;
    if (tip.done({ route: app.route, stack: app.stack, key: app.tourKey, filter: app.tourFilter, help: app.help })) app.advance();
  });

  /** The box around every element marked with the step's target, so a step can cover a chart and its controls together. */
  function find(): DOMRect | null {
    if (!tip?.target || away) return null;
    const names = Array.isArray(tip.target) ? tip.target : [tip.target];
    const visible = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-tour="${name}"]`)].map((el) => el.getBoundingClientRect()).filter((r) => r.width > 0 && r.height > 0);
    const boxes = names.map(visible).find((b) => b.length);
    if (!boxes) return null;
    const left = Math.min(...boxes.map((r) => r.left));
    const top = Math.min(...boxes.map((r) => r.top));
    return new DOMRect(left, top, Math.max(...boxes.map((r) => r.right)) - left, Math.max(...boxes.map((r) => r.bottom)) - top);
  }

  function same(a: DOMRect | null, b: DOMRect | null) {
    return a === b || (!!a && !!b && a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height);
  }

  // The target comes and goes as the person moves around and the samples change, so
  // keep looking for it while the tutorial is open.
  function remeasure() {
    const r = find();
    if (!same(r, rect)) rect = r;
  }

  $effect(() => {
    void tip;
    void away;
    void app.route;
    tick().then(() => {
      remeasure();
      const names = tip?.target ? (Array.isArray(tip.target) ? tip.target : [tip.target]) : [];
      for (const n of names) {
        const el = document.querySelector<HTMLElement>(`[data-tour="${n}"]`);
        if (el) {
          el.scrollIntoView({ block: "nearest" });
          break;
        }
      }
    });
  });

  $effect(() => {
    if (!run) return;
    const t = setInterval(remeasure, 200);
    return () => clearInterval(t);
  });

  // The card's height decides whether it fits above or below its target.
  $effect(() => {
    void tip;
    void away;
    tick().then(() => (cardH = card?.offsetHeight ?? 0));
  });

  /** Where the card goes and where its arrow points, given the target and the window. */
  const place = $derived.by(() => {
    const vw = innerW;
    const vh = innerH;
    const w = Math.min(WIDTH, vw - 2 * MARGIN);
    const h = cardH || 200;
    // Nothing to point at: out of the way in the corner, so the screen stays usable.
    const corner = { left: vw - w - MARGIN - 8, top: vh - h - 44, w, side: "none" as const, arrow: 0 };
    if (!rect) return corner;
    const r = rect;
    const clampX = (x: number) => Math.min(Math.max(MARGIN, x), vw - w - MARGIN);
    const clampY = (y: number) => Math.min(Math.max(MARGIN, y), vh - h - MARGIN);
    const cx = r.left + r.width / 2;
    const cy = r.top + r.height / 2;
    if (r.bottom + GAP + h + MARGIN <= vh) {
      const left = clampX(cx - w / 2);
      return { left, top: r.bottom + GAP, w, side: "below" as const, arrow: cx - left };
    }
    if (r.top - GAP - h - MARGIN >= 0) {
      const left = clampX(cx - w / 2);
      return { left, top: r.top - GAP - h, w, side: "above" as const, arrow: cx - left };
    }
    if (r.left - GAP - w - MARGIN >= 0) {
      const top = clampY(cy - h / 2);
      return { left: r.left - GAP - w, top, w, side: "left" as const, arrow: cy - top };
    }
    if (r.right + GAP + w + MARGIN <= vw) {
      const top = clampY(cy - h / 2);
      return { left: r.right + GAP, top, w, side: "right" as const, arrow: cy - top };
    }
    // The target fills the window: sit inside it, at the bottom, with no arrow.
    return { left: clampX(cx - w / 2), top: Math.min(r.bottom, vh) - h - MARGIN, w, side: "none" as const, arrow: 0 };
  });

  // Back on the step it was closed on: tick the box again only if they tick it.
  $effect(() => {
    if (!run) dontShow = false;
  });

  function next() {
    if (!run) return;
    if (last) app.closeTour(dontShow, true);
    else app.showTip(run.index + 1);
  }

  function back() {
    if (run && run.index > 0) app.showTip(run.index - 1);
  }

  /** Keys reach the app as usual; the tutorial only notes them, for steps that ask for one. */
  function key(e: KeyboardEvent) {
    if (!run || e.metaKey || e.ctrlKey) return;
    const target = e.target as HTMLElement;
    if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") return;
    app.tourKey = e.key;
  }

  function input(e: Event) {
    const target = e.target as HTMLInputElement;
    if (run && target.id === "filter") app.tourFilter = target.value;
  }

  /** Clicking the card mustn't leave a button focused, or Enter and Space would press it again. */
  function keepFocus(e: MouseEvent) {
    if ((e.target as HTMLElement).tagName !== "INPUT") e.preventDefault();
  }
</script>

<svelte:window bind:innerWidth={innerW} bind:innerHeight={innerH} onresize={remeasure} onkeydown={key} oninputcapture={input} />

{#if run && tip}
  {#if rect}
    <svg class="dim" width={innerW} height={innerH} aria-hidden="true">
      <path fill-rule="evenodd" d="M0 0H{innerW}V{innerH}H0Z M{rect.left - 4} {rect.top - 4}h{rect.width + 8}v{rect.height + 8}h{-(rect.width + 8)}Z" />
    </svg>
    <div class="spot" style="left:{rect.left - 4}px;top:{rect.top - 4}px;width:{rect.width + 8}px;height:{rect.height + 8}px"></div>
  {/if}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div bind:this={card} class="tip panel side-{place.side}" class:away role="dialog" tabindex="-1" aria-labelledby="tip-title" style="left:{place.left}px;top:{place.top}px;width:{place.w}px;--arrow:{place.arrow}px" onmousedown={keepFocus}>
    {#if away}
      <div class="count label">Tip {run.index + 1} of {run.tips.length} · you're exploring</div>
      <h2 id="tip-title">{note.title}</h2>
      <p class="prose">{note.body}</p>
      <footer>
        <label class="dont"><input type="checkbox" bind:checked={dontShow} onchange={(e) => e.currentTarget.blur()} /> Don't show again</label>
        <span class="spacer"></span>
        <button class="primary" onclick={() => app.showTip(run.index)}>Back to tip {run.index + 1}</button>
        <button onclick={() => app.closeTour(dontShow)}>Close</button>
      </footer>
    {:else}
      <div class="count label">{run.mode === "new" ? "What's new · " : "Tip "}{run.index + 1} of {run.tips.length}</div>
      <h2 id="tip-title">{tip.title}</h2>
      <p class="prose">{tip.body}</p>
      <footer>
        <label class="dont"><input type="checkbox" bind:checked={dontShow} onchange={(e) => e.currentTarget.blur()} /> Don't show again</label>
        <span class="spacer"></span>
        <button onclick={back} disabled={run.index === 0}>Back</button>
        <button class="primary" onclick={next}>{last ? "Done" : tip.done ? "Skip" : "Next"}</button>
        <button onclick={() => app.closeTour(dontShow)}>Close</button>
      </footer>
    {/if}
  </div>
  <div class="sample">Sample sessions. Yours come back when the tutorial closes.</div>
{/if}

<style>
  .dim {
    position: fixed;
    inset: 0;
    z-index: 21;
    pointer-events: none;
  }
  .dim path {
    fill: rgba(0, 0, 0, 0.45);
  }
  .spot {
    position: fixed;
    border: 2px solid var(--working);
    border-radius: 6px;
    z-index: 21;
    pointer-events: none;
    transition: all 0.18s ease;
  }
  .tip {
    position: fixed;
    z-index: 22;
    padding: 14px 16px 12px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.35);
  }
  .tip.away {
    border-style: dashed;
  }
  .tip::before {
    content: "";
    position: absolute;
    width: 12px;
    height: 12px;
    background: inherit;
    border: inherit;
    transform: rotate(45deg);
  }
  .side-none::before {
    display: none;
  }
  .side-below::before {
    top: -7px;
    left: clamp(14px, calc(var(--arrow) - 6px), calc(100% - 26px));
    border-right: 0;
    border-bottom: 0;
  }
  .side-above::before {
    bottom: -7px;
    left: clamp(14px, calc(var(--arrow) - 6px), calc(100% - 26px));
    border-left: 0;
    border-top: 0;
  }
  .side-left::before {
    right: -7px;
    top: clamp(14px, calc(var(--arrow) - 6px), calc(100% - 26px));
    border-left: 0;
    border-bottom: 0;
  }
  .side-right::before {
    left: -7px;
    top: clamp(14px, calc(var(--arrow) - 6px), calc(100% - 26px));
    border-right: 0;
    border-top: 0;
  }
  .count {
    margin-bottom: 4px;
  }
  h2 {
    margin: 0 0 6px;
    font-size: 15px;
  }
  p {
    margin: 0 0 12px;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
  }
  .dont {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    cursor: pointer;
  }
  .spacer {
    flex: 1;
  }
  .primary {
    background: var(--text);
    color: var(--bg);
    border-color: var(--text);
  }
  .sample {
    position: fixed;
    left: 50%;
    bottom: 3px;
    transform: translateX(-50%);
    z-index: 22;
    padding: 4px 12px;
    border-radius: 999px;
    background: var(--working);
    color: #000;
    font-weight: 600;
    white-space: nowrap;
    pointer-events: none;
  }
</style>
