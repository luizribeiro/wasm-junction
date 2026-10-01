import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { once } from "node:events";
import { tmpdir } from "node:os";
import process from "node:process";

const [browserName, ...cargoArgs] = process.argv.slice(2);
const playwrightRoot = process.env.PLAYWRIGHT_NODE_PATH;
if (!playwrightRoot) throw new Error("run this command inside `nix develop`");

const commandEnv = { ...process.env };
const diagnosticsDirectory = mkdtempSync(join(tmpdir(), "wasm-junction-browser-"));
const diagnosticsPath = join(diagnosticsDirectory, `${browserName}.log`);
process.env.DEBUG = "pw:browser";
process.env.DEBUG_FILE = diagnosticsPath;

const playwright = await import(join(playwrightRoot, "index.mjs"));
const browserType = playwright[browserName];
if (!browserType || cargoArgs.length === 0) {
  throw new Error("browser and cargo test arguments are required");
}

function delay(milliseconds) {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, milliseconds);
    timer.unref();
  });
}

async function installBrowser() {
  if (existsSync(browserType.executablePath())) return;
  const installer = spawn(process.execPath, [join(playwrightRoot, "cli.js"), "install", browserName], {
    env: commandEnv,
    stdio: "inherit",
  });
  const [code, signal] = await once(installer, "exit");
  if (code !== 0) throw new Error(`browser installation failed (${code ?? signal})`);
}

await installBrowser();

const child = spawn(
  "cargo",
  ["test", "--target", "wasm32-unknown-unknown", "--locked", ...cargoArgs],
  {
    detached: true,
    env: {
      ...commandEnv,
      CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER: "wasm-bindgen-test-runner",
      NO_HEADLESS: "1",
      WASM_BINDGEN_TEST_ADDRESS: "127.0.0.1:0",
    },
    stdio: ["ignore", "pipe", "pipe"],
  },
);

let output = "";
let settleServer;
let rejectServer;
const server = new Promise((resolve, reject) => {
  settleServer = resolve;
  rejectServer = reject;
});

function capture(chunk, destination) {
  destination.write(chunk);
  output = (output + chunk).slice(-8192);
  const match = output.match(/Interactive browsers tests are now available at (http:\/\/\S+)/);
  if (match) settleServer(match[1]);
}

child.stdout.on("data", (chunk) => capture(chunk, process.stdout));
child.stderr.on("data", (chunk) => capture(chunk, process.stderr));
child.once("error", rejectServer);
child.once("exit", (code, signal) => {
  rejectServer(new Error(`test runner exited before serving (${code ?? signal})`));
});

let browser;
let stopping = false;
let resultArrived = false;
let reportBrowserDiagnostics = false;
async function stop() {
  if (stopping) return;
  stopping = true;
  if (browser) await browser.close().catch(() => {});
  if (child.exitCode === null && child.signalCode === null) {
    process.kill(-child.pid, "SIGTERM");
    await Promise.race([once(child, "exit"), delay(3000)]);
    if (child.exitCode === null && child.signalCode === null) {
      process.kill(-child.pid, "SIGKILL");
      await once(child, "exit");
    }
  }
}

async function printBrowserDiagnostics() {
  await new Promise((resolve) => setTimeout(resolve, 50));
  const diagnostics = existsSync(diagnosticsPath)
    ? readFileSync(diagnosticsPath, "utf8").trimEnd()
    : "";
  console.error(`\n${browserName} browser diagnostics:`);
  console.error(diagnostics || "(no browser output captured)");
}

for (const [signal, status] of [["SIGINT", 130], ["SIGTERM", 143]]) {
  process.once(signal, async () => {
    await stop();
    process.exit(status);
  });
}

try {
  const url = await Promise.race([
    server,
    delay(600_000).then(() => { throw new Error("timed out waiting for the test server"); }),
  ]);
  try {
    browser = await browserType.launch({ headless: true });
  } catch (error) {
    reportBrowserDiagnostics = true;
    throw error;
  }
  browser.on("disconnected", () => {
    if (!resultArrived && !stopping) reportBrowserDiagnostics = true;
  });
  console.log(`${browserName} ${browser.version()} (Playwright 1.63.0)`);
  const page = await browser.newPage();
  page.on("console", (message) => console.log(message.text()));
  page.on("pageerror", (error) => console.error(`page error: ${error.message}`));
  await page.goto(url);
  await page.waitForFunction(
    () => document.querySelector("#output")?.textContent.includes("test result: "),
    undefined,
    { timeout: 180_000 },
  );
  const result = await page.locator("#output").textContent();
  resultArrived = true;
  process.stdout.write(`${result}\n`);
  const summary = result.match(/test result: ok\.\s+(\d+) passed;/);
  if (!summary) throw new Error(`${browserName} tests did not report a passing result`);
  if (Number(summary[1]) === 0) throw new Error(`${browserName} ran zero tests`);
} catch (error) {
  if (browser && !resultArrived && !browser.isConnected()) reportBrowserDiagnostics = true;
  throw error;
} finally {
  try {
    await stop();
  } finally {
    if (reportBrowserDiagnostics) await printBrowserDiagnostics();
    rmSync(diagnosticsDirectory, { recursive: true, force: true });
  }
}
