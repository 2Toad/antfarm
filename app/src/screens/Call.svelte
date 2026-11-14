<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { command } from "../lib/bridge";
  import { clock, dur, ms } from "../lib/format";
  import type { CallLine, Category, ResultStatus } from "../lib/types";
  import Tool from "../components/Tool.svelte";

  interface Detail { tool: string; category: Category; summary: string; status: ResultStatus; started: string | null; finished: string | null; elapsed: number | null; background: boolean; input: string | null; why: string | null; background_run: { task_id: string; output: string; finished: string | null; status: string | null } | null }
  interface Body { session: string; session_name: string; agent: string; agent_name: string; call: string; detail: Detail; output_tail: string | null; last_completed: CallLine | null }

  const body = $derived(app.body<Body>("call"));
  let tab = $state<"output" | "input" | "why" | "raw">("output");

  const command_text = $derived.by(() => {
    if (!body?.detail.input) return null;
    try {
      return (JSON.parse(body.detail.input) as { command?: string }).command ?? null;
    } catch {
      return null;
    }
  });
  const running = $derived(body ? body.detail.status === "pending" || (body.detail.background_run != null && body.detail.background_run.finished == null) : false);
  const verdict = $derived(!body ? "" : running ? "RUNNING" : body.detail.status === "error" ? "ERROR" : "OK");

  function copy() {
    const text = command_text ?? body?.detail.summary;
    if (text) command("copy", { text }).then((m) => app.say(m));
  }
</script>

{#if body}
  <div class="call">
    <div class="head">
      <span class="state s-{verdict}">{verdict}</span>
      <h1><Tool name={body.detail.tool} /></h1>
      <span class="dim">{body.detail.background ? "background" : "foreground"} · {body.detail.category}</span>
      <b class:c-Working={running}>{dur(body.detail.elapsed)}</b>
      <span class="spacer"></span>
      <button onclick={copy}>Copy command <kbd>C</kbd></button>
      <button onclick={() => app.run("go_to_terminal", { session: body.session })}>Go to terminal <kbd>G</kbd></button>
    </div>
    <pre class="cmd">{command_text ? "$ " + command_text : body.detail.summary}</pre>
    <div class="meta dim">
      <span>{body.agent_name} in {body.session_name}</span>
      <span>started {clock(body.detail.started)}</span>
      <span>finished {clock(body.detail.finished)}</span>
      {#if body.detail.background_run}<span>background task {body.detail.background_run.task_id}{body.detail.background_run.status ? ` · ${body.detail.background_run.status}` : ""}</span>{/if}
    </div>
    <div class="tabs" role="tablist" data-tour="call-tabs">
      {#each [["output", "Output"], ["input", "Input"], ["why", "Why this call"], ["raw", "Raw event"]] as [id, label]}
        <button role="tab" aria-selected={tab === id} class:on={tab === id} onclick={() => (tab = id as typeof tab)}>{label}{id === "output" && body.output_tail != null && running ? " live" : ""}</button>
      {/each}
    </div>
    <div class="body scroll">
      {#if tab === "output"}
        {#if body.output_tail != null}
          <div class="tailhead dim">{body.detail.background_run?.output}</div>
          <pre class="out">{body.output_tail || "(no output yet)"}</pre>
        {:else if running}
          <div class="empty"><span class="c-Working">● running · no output until the call returns</span><b class="big c-Working">{dur(body.detail.elapsed)}</b><p class="prose dim">Foreground commands report their output when they finish. antfarm keeps only the result's size and status, not its text.</p></div>
        {:else}
          <div class="empty"><span>{body.detail.status === "error" ? "✕ exit error" : "ok"} · {ms(body.detail.elapsed == null ? null : body.detail.elapsed * 1000)}</span><p class="prose dim">antfarm keeps a call's result size and status, not its text. Background commands show their output file here.</p></div>
        {/if}
      {:else if tab === "input"}
        <pre class="out">{body.detail.input ?? "The input is only kept for an agent's most recent calls."}</pre>
      {:else if tab === "why"}
        {#if body.detail.why}<p class="prose why">“{body.detail.why}”</p>{:else}<p class="prose dim">The agent wrote nothing just before this call, or it is no longer kept.</p>{/if}
      {:else}
        <pre class="out">{JSON.stringify({ call: body.call, agent: body.agent, session: body.session, ...body.detail, input: undefined }, null, 2)}</pre>
      {/if}
    </div>
    {#if body.last_completed}
      <div class="last dim">last completed <Tool name={body.last_completed.tool} /> {body.last_completed.target} <b class:err={body.last_completed.status === "error"}>{body.last_completed.status === "error" ? "exit error" : "ok"}</b> {ms(body.last_completed.elapsed)}</div>
    {/if}
  </div>
{:else}
  <p class="dim" style="padding:20px">This call is no longer in the agent's recent history.</p>
{/if}

<style>
  .call {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 16px 20px 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  h1 {
    margin: 0;
    font-size: 20px;
  }
  .state {
    font-weight: 700;
    padding: 2px 8px;
    border-radius: 4px;
    color: #111;
  }
  .s-RUNNING {
    background: var(--working);
  }
  .s-OK {
    background: var(--finished);
  }
  .s-ERROR {
    background: var(--error);
  }
  .spacer {
    flex: 1;
  }
  .cmd {
    font: 14px var(--mono);
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 12px 14px;
    margin: 12px 0 8px;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .meta {
    display: flex;
    gap: 22px;
    margin-bottom: 10px;
  }
  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--line);
  }
  .tabs button {
    border: 0;
    border-radius: 0;
    background: none;
    padding: 8px 14px;
    color: var(--text-3);
  }
  .tabs .on {
    color: var(--text);
    border-bottom: 2px solid var(--text);
  }
  .body {
    flex: 1;
    min-height: 0;
    padding: 12px 0;
  }
  .out {
    font: 12px/1.5 var(--mono);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
  }
  .tailhead {
    margin-bottom: 6px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 60px 20px;
    text-align: center;
  }
  .big {
    font-size: 40px;
  }
  .why {
    max-width: 820px;
  }
  .last {
    padding: 10px 0;
    border-top: 1px solid var(--line);
  }
  .err {
    color: var(--error);
  }
</style>
