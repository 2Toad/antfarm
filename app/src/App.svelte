<script lang="ts">
  import "@fontsource-variable/geist";
  import "@fontsource-variable/geist-mono";
  import "./styles/tokens.css";
  import { onMount } from "svelte";
  import { app } from "./lib/app.svelte";
  import TopBar from "./components/TopBar.svelte";
  import NeedsYou from "./components/NeedsYou.svelte";
  import Footer from "./components/Footer.svelte";
  import Palette from "./components/Palette.svelte";
  import Help from "./components/Help.svelte";
  import Machine from "./screens/Machine.svelte";
  import Project from "./screens/Project.svelte";
  import Session from "./screens/Session.svelte";

  let machine: Machine | undefined = $state();

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

  function onKey(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") {
      if (e.key === "Escape") target.blur();
      return;
    }
    if (app.help) {
      app.help = false;
      e.preventDefault();
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      app.palette = true;
      e.preventDefault();
      return;
    }
    if (app.palette || e.metaKey || e.ctrlKey) return;
    if (app.route.screen === "machine" && machine?.key(e)) {
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
        queueMicrotask(() => document.getElementById("filter")?.focus());
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
      <Session />
    {:else}
      <p class="dim" style="padding:20px">{app.route.screen} screen</p>
    {/if}
  </main>
  <Footer />
</div>
<Palette />
<Help />

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
