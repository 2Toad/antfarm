#!/usr/bin/env node
// Writes the made-up Claude Code sessions the desktop app's tutorial runs on, as
// transcripts under fixtures/demo/. The app bundles them: when the tutorial opens, it
// writes everything before LIVE_FROM into a private Claude Code folder at once, then the
// rest at the pace it was recorded, so the sessions move while you watch. The
// app/src-tauri fixtures test also renders them, as of NOW, into app/fixtures/demo/*.json
// for the browser mock and the component tests.
//
//   node scripts/demo-sessions.mjs
//   ANTFARM_WRITE_FIXTURES=1 cargo test -p antfarm-app --test fixtures
//
// Every name, path, and command here is invented. Times are fixed so the output is
// stable. A background command's output path starts with {{TASKS}}, which the app
// replaces with its own folder.

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "fixtures", "demo");
const NOW = Date.parse("2026-11-09T10:15:00Z");
/** Where the tutorial starts: everything after this plays live. */
const LIVE_FROM = NOW - 150_000;
const VERSION = "2.1.318";

let uuidSeq = 0;
let msgSeq = 0;
let toolSeq = 0;
const uuid = () => `demo-${String(++uuidSeq).padStart(5, "0")}`;
const iso = (ms) => new Date(ms).toISOString();

class Agent {
  /** `parent` is null for the session's main agent. */
  constructor(session, id, parent, { start, model, slug }) {
    this.session = session;
    this.id = id;
    this.parent = parent;
    this.t = start;
    this.model = model;
    this.slug = slug;
    this.lines = [];
    this.last = null;
  }

  base(type) {
    const s = this.session;
    const line = {
      parentUuid: this.last,
      isSidechain: this.parent !== null,
      userType: "external",
      cwd: s.cwd,
      sessionId: s.id,
      version: VERSION,
      gitBranch: s.branch,
      entrypoint: s.entrypoint,
    };
    if (this.parent) Object.assign(line, { agentId: this.id, slug: this.slug });
    line.type = type;
    line.uuid = uuid();
    line.timestamp = iso(this.t);
    this.last = line.uuid;
    return line;
  }

  usage(out) {
    return { input_tokens: 6, cache_creation_input_tokens: 900 + out * 3, cache_read_input_tokens: 18000 + this.lines.length * 1400, output_tokens: out };
  }

  assistant(content, stop) {
    const line = this.base("assistant");
    line.requestedModel = this.model.startsWith("claude-opus") ? "opus" : "sonnet";
    line.effort = "high";
    line.message = {
      id: `msg_demo_${++msgSeq}`,
      type: "message",
      role: "assistant",
      model: this.model,
      content,
      stop_reason: stop,
      usage: this.usage(stop === "end_turn" ? 120 : 260),
    };
    this.lines.push(line);
    return line;
  }

  user(content, extra = {}) {
    const line = this.base("user");
    line.message = { role: "user", content };
    Object.assign(line, extra);
    this.lines.push(line);
    return line;
  }

  wait(secs) {
    this.t += secs * 1000;
    return this;
  }

  /** Moves the clock forward to `ms`, never back. */
  at(ms) {
    this.t = Math.max(this.t, ms);
    return this;
  }

  prompt(text) {
    this.user(text);
    return this.wait(3);
  }

  say(text) {
    this.assistant([{ type: "text", text }], "end_turn");
    return this.wait(2);
  }

  /**
   * A tool call that took `secs`, with `why` as the text before it. `open: true` leaves
   * it running, for the call the agent is on right now.
   */
  call(name, input, { result = "", secs = 2, why, error = false, open = false, toolUseResult } = {}) {
    const id = `toolu_demo_${String(++toolSeq).padStart(3, "0")}`;
    const content = [];
    if (why) content.push({ type: "text", text: why });
    content.push({ type: "tool_use", id, name, input });
    const sent = this.assistant(content, "tool_use");
    if (open) return id;
    this.wait(secs);
    const block = { tool_use_id: id, type: "tool_result", content: result };
    if (error) block.is_error = true;
    this.user([block], { toolUseResult: toolUseResult ?? (name === "Bash" ? { stdout: result, stderr: "", interrupted: false } : {}), sourceToolAssistantUUID: sent.uuid });
    this.wait(3);
    return id;
  }

  /** Starts a background sub-agent and returns it. */
  spawn(id, description, { type = "general-purpose", prompt, why, model = "claude-sonnet-5-5", slug }) {
    const toolId = this.call("Agent", { description, prompt: prompt ?? description, subagent_type: type, run_in_background: true }, {
      why,
      secs: 1,
      result: `Async agent launched successfully. Agent ID: ${id}`,
      toolUseResult: { isAsync: true, status: "async_launched", agentId: id, description, resolvedModel: model },
    });
    const child = new Agent(this.session, id, this, { start: this.t - 2500, model, slug: slug ?? id });
    child.meta = {
      agentType: type,
      isBuiltIn: true,
      description,
      toolUseId: toolId,
      spawnDepth: (this.meta?.spawnDepth ?? 0) + 1,
      requestShape: "background",
      requestNonInteractive: true,
      model: model.includes("opus") ? "opus" : "sonnet",
    };
    if (this.parent) child.meta.parentAgentId = this.id;
    child.prompt(prompt ?? description);
    this.session.agents.push(child);
    return child;
  }

  /** A command started in the background, whose output arrives as `chunks` of `[secs after start, text]`. */
  background(task, command, description, chunks, { why } = {}) {
    const start = this.t;
    this.call("Bash", { command, description, run_in_background: true }, {
      why,
      secs: 1,
      result: `Command running in background with ID: ${task}. Output is being written to: {{TASKS}}/${task}.output. You will be notified when it completes.`,
      toolUseResult: { stdout: "", stderr: "", interrupted: false, backgroundTaskId: task },
    });
    this.session.tasks[task] = chunks.map(([secs, text]) => [start + secs * 1000, text]);
  }

  /** The agent's last word, and the note its parent gets when it is done. */
  finish(text) {
    this.say(text);
    this.meta.finished = true;
    const p = this.parent;
    p.t = Math.max(p.t, this.t + 1000);
    p.user(`<task-notification><task-id>${this.id}</task-id><status>completed</status><summary>${this.meta.description}</summary></task-notification>`);
    p.wait(2);
  }
}

class Session {
  /** `source` is the app it runs in, as antfarm finds it from the process tree. */
  constructor({ id, name, cwd, branch, entrypoint, start, pid, status, waitingFor, statusFor, source, exited = false }) {
    Object.assign(this, { id, name, cwd, branch, entrypoint, pid, status, waitingFor, statusFor, source, exited });
    this.agents = [];
    /** Background command output: task id to `[ms, text]` chunks. */
    this.tasks = {};
    this.main = new Agent(this, null, null, { start, model: "claude-opus-5-5" });
  }

  write() {
    const dir = join(ROOT, this.id);
    mkdirSync(join(dir, "subagents"), { recursive: true });
    const jsonl = (lines) => lines.map((l) => JSON.stringify(l)).join("\n") + "\n";
    writeFileSync(join(dir, "main.jsonl"), jsonl(this.main.lines));
    for (const a of this.agents) {
      writeFileSync(join(dir, "subagents", `agent-${a.id}.jsonl`), jsonl(a.lines));
      writeFileSync(join(dir, "subagents", `agent-${a.id}.meta.json`), JSON.stringify(a.meta) + "\n");
    }
    const registry = {
      pid: this.pid,
      sessionId: this.id,
      cwd: this.cwd,
      name: this.name,
      entrypoint: this.entrypoint,
      status: this.status,
      statusUpdatedAt: NOW - this.statusFor * 1000,
    };
    if (this.waitingFor) registry.waitingFor = this.waitingFor;
    writeFileSync(join(dir, "registry.json"), JSON.stringify(registry, null, 2) + "\n");
    if (this.source) writeFileSync(join(dir, "source.json"), JSON.stringify(this.source, null, 2) + "\n");
    for (const [task, chunks] of Object.entries(this.tasks)) {
      mkdirSync(join(dir, "tasks"), { recursive: true });
      writeFileSync(join(dir, "tasks", `${task}.jsonl`), jsonl(chunks.map(([at, text]) => ({ at: iso(at), text }))));
    }
  }
}

const min = (m) => NOW - m * 60_000;
const STORE = "/Users/demo/code/storefront";
const BILLING = "/Users/demo/code/billing-api";

// 1. Adding Apple Pay to a shop's checkout. Three sub-agents, one with its own helper,
//    and the main agent waiting on permission to push.
const pay = new Session({ id: "demo-apple-pay", name: "apple-pay", cwd: STORE, branch: "apple-pay", entrypoint: "cli", start: min(13.5), pid: 41001, status: "waiting", waitingFor: "permission prompt", statusFor: 198, source: { app: "tmux", tmux_pane: "shop:1.0" } });
{
  const m = pay.main;
  m.prompt("Add Apple Pay to checkout. Keep the card form as the fallback, and update the checkout tests.");
  m.call("Read", { file_path: `${STORE}/package.json` }, { why: "I'll check which payment SDK the shop already uses.", result: '{ "name": "storefront", "dependencies": { "@stripe/stripe-js": "^5.2.0", "react": "^19.1.0" } }', secs: 1 });
  const map = m.spawn("a3f1c0de57b2a9d01", "Map the checkout flow", { type: "Explore", why: "Three things can run at once: mapping the flow, building the button, and the tests.", slug: "map-checkout" });
  const button = m.spawn("a5b7e2c91d04f6a28", "Build the Apple Pay button", { prompt: "Add an ApplePayButton component to src/checkout that uses Stripe's Payment Request API and falls back to the card form.", slug: "apple-pay-button" });
  const tests = m.spawn("a8d24f6b0c3e1a975", "Update the checkout tests", { prompt: "Add tests for the Apple Pay path in src/checkout/__tests__ and keep the card tests passing.", slug: "checkout-tests" });

  map.call("Glob", { pattern: "src/checkout/**/*.tsx" }, { result: "src/checkout/Checkout.tsx\nsrc/checkout/CardForm.tsx\nsrc/checkout/Summary.tsx\nsrc/checkout/usePayment.ts", secs: 1 });
  map.wait(25).call("Read", { file_path: `${STORE}/src/checkout/Checkout.tsx` }, { result: "export function Checkout() { ... }", secs: 1 });
  map.call("Read", { file_path: `${STORE}/src/checkout/usePayment.ts` }, { result: "export function usePayment() { ... }", secs: 1 });
  map.wait(40).call("Grep", { pattern: "createPaymentIntent", path: `${STORE}/src`, output_mode: "content", "-n": true }, { result: "src/checkout/usePayment.ts:18:  const intent = await createPaymentIntent(cart)\nsrc/api/payments.ts:7:export async function createPaymentIntent(cart: Cart)", secs: 2 });
  map.at(min(7.5)).finish("Checkout renders CardForm inside Checkout.tsx; usePayment creates the payment intent. The button belongs above CardForm and can reuse usePayment.");

  button.call("Read", { file_path: `${STORE}/src/checkout/CardForm.tsx` }, { result: "export function CardForm({ onPaid }: Props) { ... }", secs: 1 });
  const docs = button.spawn("a2c96e40f8b13d7a5", "Check the Payment Request docs", { type: "Explore", why: "I'll have a helper confirm how Stripe exposes Apple Pay before writing the component.", slug: "payment-request-docs" });
  docs.call("WebSearch", { query: "Stripe Payment Request Button Apple Pay React" }, { result: "stripe.com/docs/stripe-js/elements/payment-request-button", secs: 4 });
  docs.wait(30).call("WebFetch", { url: "https://stripe.com/docs/stripe-js/elements/payment-request-button", prompt: "How does canMakePayment report Apple Pay?" }, { result: "paymentRequest.canMakePayment() resolves to { applePay: true } when Apple Pay is available.", secs: 9 });
  docs.at(min(8.8)).finish("Use stripe.paymentRequest(), then canMakePayment(); show the button only when it resolves with applePay: true.");
  button.at(min(8)).call("Write", { file_path: `${STORE}/src/checkout/ApplePayButton.tsx`, content: "import { usePayment } from './usePayment'\n\nexport function ApplePayButton() {\n  // ...\n}\n" }, { why: "The docs confirm the approach. Writing the component now, reusing usePayment for the intent.", result: "File created successfully", secs: 1 });
  button.at(min(5.5)).call("Edit", { file_path: `${STORE}/src/checkout/Checkout.tsx`, old_string: "<CardForm onPaid={done} />", new_string: "<ApplePayButton onPaid={done} />\n      <CardForm onPaid={done} />" }, { why: "Showing the button above the card form, which stays as the fallback.", result: "The file has been updated successfully.", secs: 1 });
  button.at(NOW - 140_000).call("Read", { file_path: `${STORE}/src/checkout/Summary.tsx` }, { why: "Checking whether the order summary needs to know about the new method.", result: "export function Summary({ cart }: Props) { ... }", secs: 1 });
  button.at(NOW - 95_000).call("Edit", { file_path: `${STORE}/src/checkout/Summary.tsx`, old_string: "Paid with card", new_string: "Paid with {method === 'apple_pay' ? 'Apple Pay' : 'card'}" }, { why: "The summary says \"Paid with card\" whatever was used. Fixing that.", result: "The file has been updated successfully.", secs: 1 });
  button.at(NOW - 50_000).call("Bash", { command: "pnpm tsc --noEmit", description: "Type-check the storefront" }, { why: "Type-checking before handing back.", open: true });

  tests.call("Read", { file_path: `${STORE}/src/checkout/__tests__/Checkout.test.tsx` }, { result: "describe('Checkout', () => { ... })", secs: 1 });
  tests.at(min(9)).call("Edit", { file_path: `${STORE}/src/checkout/__tests__/Checkout.test.tsx`, old_string: "describe('Checkout', () => {", new_string: "describe('Checkout', () => {\n  it('offers Apple Pay when the browser supports it', async () => { ... })" }, { why: "Adding a case for browsers with Apple Pay and one without.", result: "The file has been updated successfully.", secs: 1 });
  tests.at(min(6.5)).call("Bash", { command: "pnpm vitest run src/checkout", description: "Run the checkout tests" }, { result: " FAIL  src/checkout/__tests__/Checkout.test.tsx > offers Apple Pay\n   ReferenceError: ApplePaySession is not defined", error: true, secs: 14 });
  tests.at(min(4)).call("Edit", { file_path: `${STORE}/src/test/setup.ts`, old_string: "import '@testing-library/jest-dom'", new_string: "import '@testing-library/jest-dom'\nglobalThis.ApplePaySession = { canMakePayments: () => true } as never" }, { why: "jsdom has no ApplePaySession, so the test setup needs a stub.", result: "The file has been updated successfully.", secs: 1 });
  tests.at(NOW - 110_000).call("Bash", { command: "pnpm vitest run src/checkout -t 'card'", description: "Run the card tests on their own" }, { why: "Checking the stub didn't break the card path.", result: " ✓ src/checkout/__tests__/Checkout.test.tsx (4 tests) 812ms", secs: 9 });
  tests.at(NOW - 30_000).call("Bash", { command: "pnpm vitest run src/checkout", description: "Run the checkout tests" }, { open: true });

  m.at(map.t + 30_000).call("Edit", { file_path: `${STORE}/CHANGELOG.md`, old_string: "## Unreleased", new_string: "## Unreleased\n\n- Apple Pay at checkout" }, { why: "While those run, a changelog line.", result: "The file has been updated successfully.", secs: 1 });
  m.at(min(5)).call("Edit", { file_path: `${STORE}/README.md`, old_string: "## Payments", new_string: "## Payments\n\nCheckout offers Apple Pay on supported browsers, with the card form as the fallback." }, { why: "The README lists payment methods, so it should mention Apple Pay.", result: "The file has been updated successfully.", secs: 1 });
  m.at(min(4.5)).call("Bash", { command: "git add -A && git commit -m 'Add Apple Pay to checkout'", description: "Commit the work so far" }, { result: "[apple-pay 4be19c2] Add Apple Pay to checkout\n 4 files changed, 96 insertions(+), 3 deletions(-)", secs: 1 });
  m.at(NOW - 200_000).call("Bash", { command: "git push -u origin apple-pay", description: "Push the branch" }, { why: "Pushing the branch so CI can run the full suite.", open: true });
}

// 2. Speeding up a slow API endpoint. Working, with one helper done and one running.
const perf = new Session({ id: "demo-invoice-perf", name: "invoice-perf", cwd: BILLING, branch: "main", entrypoint: "claude-vscode", start: min(12), pid: 41002, status: "busy", statusFor: 4, source: { app: "Visual Studio Code" } });
{
  const m = perf.main;
  m.prompt("GET /invoices is taking 2 seconds for big accounts. Find out why and fix it.");
  m.call("Grep", { pattern: "fn list_invoices", path: `${BILLING}/src`, output_mode: "files_with_matches" }, { result: "src/routes/invoices.rs", secs: 1 });
  m.call("Read", { file_path: `${BILLING}/src/routes/invoices.rs` }, { result: "pub async fn list_invoices(...) { ... }", secs: 1 });
  const prof = m.spawn("a6e03b9d2f7c41a58", "Profile the invoices endpoint", { prompt: "Time GET /invoices against the seeded big account and report where the time goes.", slug: "profile-invoices" });
  const queries = m.spawn("a9c15d7e3b2f80a46", "Find slow invoice queries", { type: "Explore", prompt: "Find every query GET /invoices runs and flag any that run once per invoice.", slug: "slow-queries" });
  prof.call("Bash", { command: "cargo run --release --bin seed -- --account big", description: "Seed a big account" }, { result: "seeded 12,400 invoices", secs: 38 });
  prof.wait(20).call("Bash", { command: "hey -n 50 http://localhost:8080/invoices?account=big", description: "Time the endpoint" }, { result: "Average: 2.1432 secs", secs: 24 });
  prof.at(min(8.5)).finish("Average 2.14s over 50 requests. Most of it is database time: about 400 queries per request.");
  queries.call("Grep", { pattern: "query_as!", path: `${BILLING}/src/db`, output_mode: "content", "-n": true }, { result: "src/db/invoices.rs:22\nsrc/db/line_items.rs:9", secs: 2 });
  queries.at(min(7)).call("Read", { file_path: `${BILLING}/src/db/line_items.rs` }, { result: "pub async fn for_invoice(id: i64) -> Vec<LineItem>", secs: 1 });
  queries.at(NOW - 20_000).call("Grep", { pattern: "line_items::for_invoice", path: `${BILLING}/src`, output_mode: "content", "-n": true }, { why: "That looks like one query per invoice. Checking who calls it.", open: true });
  m.at(prof.t + 40_000).call("Edit", { file_path: `${BILLING}/src/db/line_items.rs`, old_string: "pub async fn for_invoice(", new_string: "pub async fn for_invoices(ids: &[i64]) -> Vec<LineItem> { ... }\n\npub async fn for_invoice(" }, { why: "400 queries a request means line items load one invoice at a time. Adding a batched loader.", result: "The file has been updated successfully.", secs: 1 });
  m.at(min(4)).call("Edit", { file_path: `${BILLING}/src/routes/invoices.rs`, old_string: "for inv in &invoices {", new_string: "let items = line_items::for_invoices(&ids).await?;\n    for inv in &invoices {" }, { result: "The file has been updated successfully.", secs: 1 });
  m.at(NOW - 140_000).background("bk7f2a", "cargo bench --bench invoices", "Benchmark the invoices endpoint", [
    [2, "   Compiling billing-api v0.4.0 (/Users/demo/code/billing-api)\n"],
    [21, "    Finished `bench` profile [optimized] target(s) in 19.40s\n"],
    [22, "     Running benches/invoices.rs (target/release/deps/invoices-5c1d2e)\n"],
    [25, "list_invoices/small     time:   [3.912 ms 3.948 ms 3.990 ms]\n"],
    [48, "list_invoices/medium    time:   [41.20 ms 41.77 ms 42.31 ms]\n"],
    [95, "list_invoices/big       time:   [212.4 ms 214.9 ms 217.6 ms]\n                        change: [-90.112% -89.973% -89.830%] (p = 0.00 < 0.05)\n                        Performance has improved.\n"],
    [140, "list_invoices/big_filtered\n                        time:   [188.1 ms 190.3 ms 192.8 ms]\n"],
    [210, "list_invoices/huge      time:   [1.204 s 1.219 s 1.233 s]\n"],
    [300, "\nBenchmarks finished.\n"],
  ], { why: "Starting the benchmark in the background while the tests run." });
  const loaderTests = m.at(NOW - 100_000).spawn("a4d0e8b2c6f19a37e", "Test the batched loader", { prompt: "Add tests for line_items::for_invoices: empty input, one invoice, and invoices with no line items.", slug: "loader-tests" });
  loaderTests.call("Read", { file_path: `${BILLING}/src/db/line_items.rs` }, { result: "pub async fn for_invoices(ids: &[i64]) -> Vec<LineItem> { ... }", secs: 1 });
  loaderTests.at(NOW - 70_000).call("Write", { file_path: `${BILLING}/tests/line_items.rs`, content: "#[sqlx::test]\nasync fn batches_line_items(pool: PgPool) { ... }\n" }, { why: "Three cases: no ids, one invoice, and an invoice without line items.", result: "File created successfully", secs: 1 });
  loaderTests.at(NOW - 15_000).call("Bash", { command: "cargo test --test line_items", description: "Run the new loader tests" }, { open: true });
  m.at(NOW - 64_000).call("Bash", { command: "cargo test invoices", description: "Run the invoice tests" }, { why: "Running the invoice tests against the batched loader.", open: true });
}

// 3. A quiet session that finished its docs change and is waiting for the next prompt.
const docs = new Session({ id: "demo-docs", name: "readme-refresh", cwd: STORE, branch: "docs", entrypoint: "claude-vscode", start: min(26), pid: 41003, status: "idle", statusFor: 4.6 * 60, source: { app: "Visual Studio Code" } });
{
  const m = docs.main;
  m.prompt("Update the README's setup steps for pnpm 10.");
  m.at(min(6)).call("Read", { file_path: `${STORE}/README.md` }, { result: "# storefront\n\n## Setup\n\nnpm install", secs: 1 });
  m.at(min(4.8)).call("Edit", { file_path: `${STORE}/README.md`, old_string: "npm install", new_string: "corepack enable\npnpm install" }, { why: "Swapping npm for pnpm through corepack.", result: "The file has been updated successfully.", secs: 1 });
  m.say("The setup steps now use pnpm 10 through corepack.");
}

// 4. Waiting on a question: which limit the free plan should get. Claude Code writes the
//    question to the transcript only once it is answered, so the registry says
//    "input needed" and the last thing the agent said is the question.
const limits = new Session({ id: "demo-rate-limits", name: "rate-limits", cwd: BILLING, branch: "rate-limits", entrypoint: "cli", start: min(19), pid: 41005, status: "waiting", waitingFor: "input needed", statusFor: 330, source: { app: "Ghostty" } });
{
  const m = limits.main;
  m.prompt("Add per-plan rate limits to the public API.");
  m.call("Read", { file_path: `${BILLING}/src/plans.rs` }, { why: "First, what plans exist.", result: "pub enum Plan { Free, Team, Enterprise }", secs: 1 });
  const routes = m.spawn("a7b3e9c1d5f20864b", "List the public routes", { type: "Explore", prompt: "List every route under /v1 and which plan can call it.", slug: "public-routes" });
  routes.call("Grep", { pattern: "\\.route\\(\"/v1", path: `${BILLING}/src`, output_mode: "content", "-n": true }, { result: "src/routes/mod.rs:14:  .route(\"/v1/invoices\", get(list_invoices))\nsrc/routes/mod.rs:15:  .route(\"/v1/customers\", get(list_customers))", secs: 2 });
  routes.at(min(12)).finish("Two public routes, /v1/invoices and /v1/customers. Every plan can call both.");
  m.at(min(10)).call("Write", { file_path: `${BILLING}/src/middleware/rate_limit.rs`, content: "pub struct RateLimit { per_minute: u32 }\n" }, { why: "A middleware that reads the caller's plan and counts requests per minute.", result: "File created successfully", secs: 1 });
  m.at(min(7)).call("Edit", { file_path: `${BILLING}/src/routes/mod.rs`, old_string: "Router::new()", new_string: "Router::new().layer(RateLimit::for_plans())" }, { result: "The file has been updated successfully.", secs: 1 });
  m.at(min(5.6)).say("Free plan: 60 or 100 requests a minute? The pricing page says 60, but the API docs promise 100. Team and Enterprise get 1,000 and 10,000 either way.");
}

// 5. A session that ended a while ago. Its process is gone, so antfarm lists it as exited.
const lint = new Session({ id: "demo-lint", name: "lint-cleanup", cwd: STORE, branch: "lint", entrypoint: "cli", start: min(48), pid: 41004, status: "idle", statusFor: 30 * 60, exited: true });
{
  const m = lint.main;
  m.prompt("Fix the lint warnings in src/checkout.");
  m.call("Bash", { command: "pnpm eslint src/checkout", description: "Lint the checkout" }, { result: "src/checkout/CardForm.tsx\n  12:7  warning  'unused' is assigned a value but never used", secs: 6 });
  m.call("Edit", { file_path: `${STORE}/src/checkout/CardForm.tsx`, old_string: "  const unused = 1\n", new_string: "" }, { result: "The file has been updated successfully.", secs: 1 });
  m.at(min(31)).say("No lint warnings left in src/checkout.");
}

rmSync(ROOT, { recursive: true, force: true });
const sessions = [pay, perf, docs, limits, lint];
for (const s of sessions) s.write();
// NOW is the moment the rendered fixtures show; LIVE_FROM is where the tutorial starts.
writeFileSync(join(ROOT, "demo.json"), JSON.stringify({ now: iso(NOW), liveFrom: iso(LIVE_FROM), exited: sessions.filter((s) => s.exited).map((s) => s.pid) }, null, 2) + "\n");
console.log(`wrote ${ROOT}`);
