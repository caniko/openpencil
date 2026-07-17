import { test, expect } from "bun:test";
import { buildBootHtml, buildWebviewHtml, originOf } from "./webview-shell";

test("originOf extracts scheme://host:port", () => {
  expect(originOf("http://127.0.0.1:45001/")).toBe("http://127.0.0.1:45001");
  expect(originOf("http://127.0.0.1:45001/index.html?x=1")).toBe("http://127.0.0.1:45001");
});

test("boot HTML reports window.origin via op-shell/ready and has no iframe", () => {
  const html = buildBootHtml("N0NCE");
  expect(html).toContain("op-shell/ready");
  expect(html).toContain("window.origin");
  expect(html).not.toContain("<iframe");
  // boot CSP has script-src but no frame-src (no iframe yet).
  expect(html).toContain("script-src 'nonce-N0NCE'");
  expect(html).not.toContain("frame-src");
  // nonce is applied to the script tag.
  expect(html).toContain('<script nonce="N0NCE">');
});

test("full HTML embeds the iframe with frame-src pinned to the daemon origin", () => {
  const html = buildWebviewHtml({ iframeSrc: "http://127.0.0.1:45010/", nonce: "N1" });
  expect(html).toContain('src="http://127.0.0.1:45010/"');
  expect(html).toContain("frame-src http://127.0.0.1:45010;");
  expect(html).toContain('<script nonce="N1">');
});

test("full HTML forwards to the iframe with an explicit origin, never '*'", () => {
  const html = buildWebviewHtml({ iframeSrc: "http://127.0.0.1:45010/", nonce: "N1" });
  // No wildcard postMessage target anywhere.
  expect(/postMessage\([^)]*,\s*["']\*["']\s*\)/.test(html)).toBe(false);
  // The forward uses the pinned origin constant.
  expect(html).toContain("frame.contentWindow.postMessage(e.data, IFRAME_ORIGIN)");
  expect(html).toContain('IFRAME_ORIGIN = "http://127.0.0.1:45010"');
});

test("full HTML enforces both source and origin on inbound page messages", () => {
  const html = buildWebviewHtml({ iframeSrc: "http://127.0.0.1:45010/", nonce: "N1" });
  expect(html).toContain("e.source === frame.contentWindow && e.origin === IFRAME_ORIGIN");
});

test("full HTML guards on typeof string and does not forward control messages", () => {
  const html = buildWebviewHtml({ iframeSrc: "http://127.0.0.1:45010/", nonce: "N1" });
  expect(html).toContain('typeof e.data !== "string"');
  expect(html).toContain('e.data.indexOf("op-shell/") !== -1');
});

test("nonce is not reused across boot and full unless the caller reuses it", () => {
  // The builders are pure — they use whatever nonce they're given. Distinct
  // nonces produce distinct script tags.
  expect(buildBootHtml("A")).toContain('nonce="A"');
  expect(buildWebviewHtml({ iframeSrc: "http://x.y:1/", nonce: "B" })).toContain('nonce="B"');
});
