import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const ROOT = process.cwd();

// 1. Verify production bundle exists and contains expected app shell
const distHtml = fs.readFileSync(path.join(ROOT, "dist/index.html"), "utf-8");
assert.match(distHtml, /<title>OpenViking Desktop<\/title>/, "dist/index.html must include app title");
assert.match(distHtml, /assets\/index-.*\.js/, "dist/index.html must reference built JS bundle");

// 2. Boot Vite preview directly (no npx wrapper) and verify HTTP 200 serving of HTML + JS bundle
const port = 4173 + Math.floor(Math.random() * 500);
const viteBin = path.join(ROOT, "node_modules/vite/bin/vite.js");
const preview = spawn(process.execPath, [viteBin, "preview", "--port", String(port), "--host", "127.0.0.1"], {
  cwd: ROOT,
  stdio: "ignore",
});

async function waitForServer(url, retries = 30) {
  for (let i = 0; i < retries; i++) {
    try {
      const res = await fetch(url);
      if (res.ok) return res;
    } catch {}
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`Preview server did not start at ${url}`);
}

let exitCode = 0;
try {
  const baseUrl = `http://127.0.0.1:${port}`;
  const res = await waitForServer(baseUrl);
  const html = await res.text();
  assert.match(html, /id="root"/, "Served HTML must contain #root mount node");

  const jsMatch = html.match(/src="(\/assets\/index-[^"]+\.js)"/);
  assert.ok(jsMatch, "Could not find main JS asset path in served HTML");
  const jsRes = await fetch(`${baseUrl}${jsMatch[1]}`);
  assert.equal(jsRes.status, 200, "Main JS bundle must return HTTP 200");
  const jsCode = await jsRes.text();

  assert.match(jsCode, /Dreaming/, "Dreaming workbench must be present in bundle");
  assert.match(jsCode, /viking:\/\//, "viking:// explorer must be present in bundle");
  assert.match(jsCode, /Antigravity/, "Antigravity adapter UI must be present in bundle");
  assert.match(jsCode, /Unified Line Diff/, "Line diff view must be present in bundle");

  console.log("Frontend E2E smoke test passed:", baseUrl);
} catch (err) {
  console.error(err);
  exitCode = 1;
} finally {
  preview.kill("SIGKILL");
  process.exit(exitCode);
}
