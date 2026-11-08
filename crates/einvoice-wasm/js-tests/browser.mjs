// Runs the browser test (browser-page.mjs) in each headless browser given, offline: a
// local HTTP server on 127.0.0.1 serves the installed package, the test modules and the
// corpus documents; the page posts its result back. Exit status 0 when every browser
// passed.
//
//   node browser.mjs PACKAGE_DIR REPO BROWSER...     BROWSER: chrome=PATH or firefox=PATH
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, extname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const [packageDir, repo, ...browsers] = process.argv.slice(2);
if (!packageDir || !repo || browsers.length === 0) {
  console.error("usage: node browser.mjs PACKAGE_DIR REPO chrome=PATH|firefox=PATH...");
  process.exit(2);
}
const testsDir = dirname(fileURLToPath(import.meta.url));
const TIMEOUT_MS = 600_000;

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json",
};

/** The file `path` under `root`, or null when it would leave it. */
function under(root, path) {
  const base = resolve(root);
  const file = resolve(base, `.${sep}${decodeURIComponent(path)}`);
  return file.startsWith(base + sep) ? file : null;
}

/** Every file under `dir`, relative to `repo`, '/'-separated. */
function list(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...list(path));
    else out.push(relative(repo, path).split(sep).join("/"));
  }
  return out;
}

let resolveResult;
// The browser under test, for the progress messages.
let current = "";
const server = createServer((request, response) => {
  const url = new URL(request.url, "http://127.0.0.1");
  if (request.method === "POST" && url.pathname === "/progress") {
    let body = "";
    request.on("data", (chunk) => {
      body += chunk;
    });
    request.on("end", () => {
      response.end("ok");
      console.log(`  ${current}: ${body}`);
    });
    return;
  }
  if (request.method === "POST" && url.pathname === "/result") {
    let body = "";
    request.on("data", (chunk) => {
      body += chunk;
    });
    request.on("end", () => {
      response.end("ok");
      resolveResult?.(JSON.parse(body));
    });
    return;
  }
  let file = null;
  let text = null;
  if (url.pathname === "/") {
    text = '<!doctype html><meta charset="utf-8"><title>einvoice-validator</title>' +
      '<script type="module" src="/tests/browser-page.mjs"></script><body>running</body>';
  } else if (url.pathname === "/favicon.ico") {
    response.writeHead(204).end();
    return;
  } else if (url.pathname.startsWith("/pkg/")) {
    file = under(packageDir, url.pathname.slice("/pkg/".length));
  } else if (url.pathname.startsWith("/tests/")) {
    file = under(testsDir, url.pathname.slice("/tests/".length));
  } else if (url.pathname.startsWith("/repo/artefacts/") || url.pathname.startsWith("/repo/corpus/")) {
    file = under(repo, url.pathname.slice("/repo/".length));
  } else if (url.pathname.startsWith("/goldens/corpus/")) {
    const dir = under(repo, url.pathname.slice("/goldens/".length));
    text = dir === null ? null : list(join(dir, "golden")).join("\n");
  }
  try {
    const body = text ?? (file === null ? null : readFileSync(file));
    if (body === null) throw new Error("not found");
    const type = text !== null && url.pathname === "/" ? TYPES[".html"] : TYPES[extname(file ?? "")];
    response.writeHead(200, { "content-type": type ?? "application/octet-stream" });
    response.end(body);
  } catch {
    response.writeHead(404).end();
  }
});
await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
const { port } = server.address();
const page = `http://127.0.0.1:${port}/`;

/** The command that opens `page` in the headless browser, with a throw-away profile. */
function command(kind, path, profile) {
  if (kind === "chrome") {
    return [path, [
      "--headless=new",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-gpu",
      "--disable-extensions",
      "--disable-background-networking",
      "--disable-component-update",
      "--disable-sync",
      "--remote-debugging-port=0",
      page,
    ]];
  }
  if (kind === "firefox") {
    writeFileSync(join(profile, "user.js"), [
      'user_pref("browser.shell.checkDefaultBrowser", false);',
      'user_pref("browser.startup.homepage_override.mstone", "ignore");',
      'user_pref("datareporting.policy.dataSubmissionEnabled", false);',
      'user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);',
      'user_pref("app.update.enabled", false);',
      'user_pref("network.proxy.type", 0);',
    ].join("\n"));
    return [path, ["--headless", "--no-remote", "--profile", profile, page]];
  }
  throw new Error(`unknown browser kind ${kind} (chrome or firefox)`);
}

let failed = 0;
for (const spec of browsers) {
  const [kind, path] = spec.split("=");
  current = kind;
  const profile = mkdtempSync(join(tmpdir(), "einvoice-browser-"));
  const result = new Promise((ok) => {
    resolveResult = ok;
  });
  const [program, args] = command(kind, path, profile);
  // A process group of its own, so that the browser's helper processes are killed with it.
  const child = spawn(program, args, { stdio: ["ignore", "ignore", "pipe"], detached: true });
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  const exited = new Promise((ok) => child.on("exit", (code) => ok({ exited: code })));
  const timeout = new Promise((ok) => setTimeout(() => ok({ timeout: true }), TIMEOUT_MS).unref());
  const outcome = await Promise.race([result, exited, timeout]);
  try {
    process.kill(-child.pid, "SIGKILL");
  } catch {
    // The group is gone already.
  }
  await exited;
  // The browser's helper processes may still be writing to the profile for a moment.
  rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  if (outcome.timeout || outcome.exited !== undefined) {
    failed += 1;
    console.error(`${kind}: ${outcome.timeout ? "no result in time" : `exited (${outcome.exited}) without a result`}`);
    console.error(stderr.split("\n").slice(-20).join("\n"));
    continue;
  }
  for (const line of outcome.log) console.log(`${kind}: ${line}`);
  if (outcome.failures.length > 0) {
    failed += 1;
    for (const f of outcome.failures) console.error(`${kind}: FAILED ${f}`);
  } else {
    console.log(`${kind}: ok`);
  }
}
server.close();
process.exit(failed === 0 ? 0 : 1);
