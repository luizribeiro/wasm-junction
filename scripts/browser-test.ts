import type { Buffer } from "node:buffer";
import { execFileSync, spawn } from "node:child_process";
import { once } from "node:events";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import process from "node:process";
import type { Browser } from "playwright-core";

const [browserName, ...cargoArgs] = process.argv.slice(2);
const configuredPlaywrightRoot = process.env.PLAYWRIGHT_NODE_PATH;
if (!configuredPlaywrightRoot) throw new Error("run this command inside `nix develop`");
if (browserName !== "chromium" && browserName !== "firefox" && browserName !== "webkit") {
  throw new Error("browser and cargo test arguments are required");
}
if (cargoArgs.length === 0) throw new Error("browser and cargo test arguments are required");
const playwrightRoot = configuredPlaywrightRoot;
const selectedBrowserName = browserName;

const commandEnv = { ...process.env };
if (cargoArgs[0] === "--workspace-tests") {
  if (cargoArgs.length !== 1) throw new Error("--workspace-tests takes no arguments");
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
      encoding: "utf8",
      env: commandEnv,
    }),
  ) as {
    packages: Array<{
      name: string;
      dependencies: Array<{ name: string }>;
      targets: Array<{ kind: string[]; name: string; src_path: string; test: boolean }>;
    }>;
  };

  const browserTargets = metadata.packages.flatMap(pkg => {
    if (!pkg.dependencies.some(dependency => dependency.name === "wasm-bindgen-test")) return [];
    return pkg.targets
      .filter(target => {
        if (!target.test) return false;
        if (target.kind.includes("lib")) return treeContainsBrowserTests(dirname(target.src_path));
        return target.kind.includes("test") && fileContainsBrowserTests(target.src_path);
      })
      .map(target => ({ packageName: pkg.name, target }));
  });
  if (browserTargets.length === 0) throw new Error("the workspace has no browser test targets");
  for (const { packageName, target } of browserTargets) {
    const selector = target.kind.includes("lib") ? ["--lib"] : ["--test", target.name];
    execFileSync(
      process.execPath,
      [import.meta.filename, selectedBrowserName, "-p", packageName, ...selector],
      {
        env: commandEnv,
        stdio: "inherit",
      },
    );
  }
  process.exit(0);
}

function fileContainsBrowserTests(path: string): boolean {
  return readFileSync(path, "utf8").includes("wasm_bindgen_test");
}

function treeContainsBrowserTests(directory: string): boolean {
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) {
      if (treeContainsBrowserTests(path)) return true;
    } else if (path.endsWith(".rs") && fileContainsBrowserTests(path)) {
      return true;
    }
  }
  return false;
}
const diagnosticsDirectory = mkdtempSync(join(tmpdir(), "wasm-junction-browser-"));
const diagnosticsPath = join(diagnosticsDirectory, `${selectedBrowserName}.log`);
process.env.DEBUG = "pw:browser";
process.env.DEBUG_FILE = diagnosticsPath;
if (process.platform === "linux") {
  // Playwright checks libraries with the `ldd` on PATH, which in the dev shell searches only the
  // Nix store; the browsers load system libraries, so a real gap still fails at launch.
  process.env.PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = "1";
}

// The runtime path and declarations both come from the flake's playwright-driver package.
const playwright: typeof import("playwright-core") = await import(
  join(playwrightRoot, "index.mjs")
);
const browserType = playwright[selectedBrowserName];

// The dev shell points XDG_DATA_DIRS at the Nix store only, which hides the system's GSettings
// schemas; WebKit's network process aborts without them.
const browserEnv = { ...process.env };
delete browserEnv.XDG_DATA_DIRS;

function delay(milliseconds: number): Promise<void> {
  return new Promise(resolve => {
    const timer = setTimeout(resolve, milliseconds);
    timer.unref();
  });
}

async function installBrowser(): Promise<void> {
  if (existsSync(browserType.executablePath())) return;
  const installer = spawn(
    process.execPath,
    [join(playwrightRoot, "cli.js"), "install", selectedBrowserName],
    {
      env: commandEnv,
      stdio: "inherit",
    },
  );
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
const {
  promise: server,
  resolve: settleServer,
  reject: rejectServer,
} = Promise.withResolvers<string>();

function capture(chunk: Buffer, destination: NodeJS.WritableStream): void {
  destination.write(chunk);
  output = (output + chunk).slice(-8192);
  const match = output.match(/Interactive browsers tests are now available at (http:\/\/\S+)/);
  if (match) settleServer(match[1]);
}

child.stdout.on("data", chunk => capture(chunk, process.stdout));
child.stderr.on("data", chunk => capture(chunk, process.stderr));
child.once("error", rejectServer);
child.once("exit", (code, signal) => {
  rejectServer(new Error(`test runner exited before serving (${code ?? signal})`));
});

let browser: Browser | undefined;
let stopping = false;
let resultArrived = false;
let reportBrowserDiagnostics = false;
async function stop(): Promise<void> {
  if (stopping) return;
  stopping = true;
  if (browser) await browser.close().catch(() => {});
  if (child.exitCode === null && child.signalCode === null) {
    if (child.pid !== undefined) process.kill(-child.pid, "SIGTERM");
    await Promise.race([once(child, "exit"), delay(3000)]);
    if (child.exitCode === null && child.signalCode === null) {
      if (child.pid !== undefined) process.kill(-child.pid, "SIGKILL");
      await once(child, "exit");
    }
  }
}

async function printBrowserDiagnostics(): Promise<void> {
  await new Promise(resolve => setTimeout(resolve, 50));
  const diagnostics = existsSync(diagnosticsPath)
    ? readFileSync(diagnosticsPath, "utf8").trimEnd()
    : "";
  console.error(`\n${selectedBrowserName} browser diagnostics:`);
  console.error(diagnostics || "(no browser output captured)");
}

for (const [signal, status] of [
  ["SIGINT", 130],
  ["SIGTERM", 143],
] as const) {
  process.once(signal, async () => {
    await stop();
    process.exit(status);
  });
}

try {
  const url = await Promise.race([
    server,
    delay(600_000).then(() => {
      throw new Error("timed out waiting for the test server");
    }),
  ]);
  const launchedBrowser = await browserType.launch({ headless: true, env: browserEnv });
  browser = launchedBrowser;
  console.log(`${selectedBrowserName} ${launchedBrowser.version()} (Playwright 1.63.0)`);
  const page = await launchedBrowser.newPage();
  page.on("console", message => console.log(message.text()));
  page.on("pageerror", error => console.error(`page error: ${error.message}`));
  await page.goto(url);
  // Keep this a string because the runner's Node and WebWorker types don't declare document.
  await page.waitForFunction(
    'document.querySelector("#output")?.textContent.includes("test result: ")',
    undefined,
    { timeout: 180_000 },
  );
  const result = await page.locator("#output").textContent();
  if (result === null) throw new Error(`${selectedBrowserName} returned no test output`);
  resultArrived = true;
  process.stdout.write(`${result}\n`);
  const summary = result.match(/test result: ok\.\s+(\d+) passed;/);
  if (!summary) throw new Error(`${selectedBrowserName} tests did not report a passing result`);
  if (Number(summary[1]) === 0) throw new Error(`${selectedBrowserName} ran zero tests`);
} catch (error) {
  reportBrowserDiagnostics = !resultArrived;
  throw error;
} finally {
  try {
    await stop();
  } finally {
    if (reportBrowserDiagnostics) await printBrowserDiagnostics();
    rmSync(diagnosticsDirectory, { recursive: true, force: true });
  }
}
