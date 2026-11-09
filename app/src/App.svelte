<script lang="ts">
  import "@fontsource-variable/geist";
  import "@fontsource-variable/geist-mono";
  import "./styles/tokens.css";
  import { onMount, tick } from "svelte";
  import { app } from "./lib/app.svelte";
  import TopBar from "./components/TopBar.svelte";
  import NeedsYou from "./components/NeedsYou.svelte";
  import Footer from "./components/Footer.svelte";
  import Palette from "./components/Palette.svelte";
  import Help from "./components/Help.svelte";
  import Tour from "./components/Tour.svelte";
  import Machine from "./screens/Machine.svelte";
  import Project from "./screens/Project.svelte";
  import Session from "./screens/Session.svelte";
  import Lanes from "./screens/Lanes.svelte";
  import Agent from "./screens/Agent.svelte";
  import Call from "./screens/Call.svelte";

  let machine: Machine | undefined = $state();
  let lanes: Lanes | undefined = $state();
  let agent: Agent | undefined = $state();
  let sessionScreen: Session | undefined = $state();

  const SORTS = ["needs_you", "cost", "name"] as const;
  const SORT_LABEL = { needs_you: "needs you first", cost: "cost", name: "name" };

  /** The whole command a waiting session wants to run, for C. */
  function waitingCommand(id: string): string | null {
    const b = app.view?.body as { screen: string; id?: string; waiting?: { tool: string; request: string } | null } | null;
    if (b?.screen === "session" && b.id === id && b.waiting?.tool) return b.waiting.request;
    const w = app.view?.header.needs_you.find((n) => n.session === id);
    return w ? (w.command ?? null) : null;
  }

  onMount(() => {
    app.start();
  });

  /** The session the screen is about, for G, C, Z, M, and L. */
  function focusedSession(): string | null {
    const r = app.route;
    if ("id" in r) return r.id;
    if ("session" in r) return r.session;
    return machine?.selectedSession() ?? null;
  }

  /** S, T, f, and a: the same table choices as the terminal UI. */
  function prefKey(e: KeyboardEvent): boolean {
    const p = app.prefs;
    switch (e.key) {
      case "s":
      case "S": {
        const sort = SORTS[(SORTS.indexOf(p.sort) + 1) % SORTS.length]!;
        app.setPrefs({ sort }, `sorted by ${SORT_LABEL[sort]}`);
        return true;
      }
      case "t":
      case "T":
        app.setPrefs({ tree: !p.tree }, p.tree ? "sub-agents hidden" : "sub-agents shown");
        return true;
      case "f":
        app.setPrefs({ show_finished: !p.show_finished }, p.show_finished ? "finished sub-agents hidden" : "finished sub-agents shown");
        return true;
      case "a":
        app.setPrefs({ show_exited: !p.show_exited }, p.show_exited ? "exited sessions hidden" : "exited sessions shown");
        return true;
    }
    return false;
  }

  function onKey(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    // ⌘K works from anywhere, the filter box included.
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k" && !app.palette) {
      target.blur?.();
      app.help = false;
      app.palette = true;
      e.preventDefault();
      return;
    }
    if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") {
      if (e.key === "Escape") target.blur();
      return;
    }
    if (app.help) {
      app.help = false;
      e.preventDefault();
      return;
    }
    if (app.palette || e.metaKey || e.ctrlKey) return;
    if (app.route.screen === "lanes" && lanes?.key(e)) {
      e.preventDefault();
      return;
    }
    if (app.route.screen === "agent" && agent?.key(e)) {
      e.preventDefault();
      return;
    }
    if (app.route.screen === "machine" && machine?.key(e)) {
      e.preventDefault();
      return;
    }
    if (app.route.screen === "session" && sessionScreen?.key(e)) {
      e.preventDefault();
      return;
    }
    if (app.route.screen === "machine" && prefKey(e)) {
      e.preventDefault();
      return;
    }
    const session = focusedSession();
    switch (e.key) {
      case "Escape":
        app.back();
        break;
      case "?":
        app.help = true;
        break;
      case "/":
        app.machineMode = "table";
        if (app.route.screen !== "machine") app.home();
        tick().then(() => document.getElementById("filter")?.focus());
        break;
      case "n":
      case "N": {
        const waiting = app.view?.header.needs_you ?? [];
        if (!waiting.length) {
          app.say("nothing is waiting for you");
          break;
        }
        const i = waiting.findIndex((w) => w.session === session);
        app.go({ screen: "session", id: waiting[(i + 1) % waiting.length]!.session });
        break;
      }
      case "g":
      case "G":
        if (session) app.run("go_to_terminal", { session });
        break;
      case "c":
      case "C": {
        const text = session ? waitingCommand(session) : null;
        if (text) app.run("copy", { text });
        else app.say("no command to copy");
        break;
      }
      case "z":
      case "Z":
        if (session) app.run("snooze", { session });
        break;
      case "m":
      case "M":
        if (session) app.run("mute", { session });
        break;
      case "l":
      case "L":
        if (session && app.route.screen !== "lanes") app.go({ screen: "lanes", id: session });
        break;
      default:
        return;
    }
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="frame">
  <TopBar />
  <NeedsYou except={app.route.screen === "session" ? (app.route as { id: string }).id : null} />
  <main>
    {#if app.route.screen === "machine"}
      <Machine bind:this={machine} />
    {:else if app.route.screen === "project"}
      <Project />
    {:else if app.route.screen === "session"}
      <Session bind:this={sessionScreen} />
    {:else if app.route.screen === "lanes"}
      <Lanes bind:this={lanes} />
    {:else if app.route.screen === "agent"}
      <Agent bind:this={agent} />
    {:else if app.route.screen === "call"}
      <Call />
    {/if}
  </main>
  <Footer />
</div>
<Palette />
<Help />
<Tour />

<style>
  .frame {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  main {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
</style>
