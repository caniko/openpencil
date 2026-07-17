import { test, expect } from "bun:test";
import { isShellControl, parseShellReadyOrigin } from "./shell-messages";

test("isShellControl matches only a top-level op-shell/ type", () => {
  expect(isShellControl(JSON.stringify({ type: "op-shell/ready", origin: "x" }))).toBe(true);
  expect(isShellControl(JSON.stringify({ type: "op-bridge/snapshot-result", requestId: "r" }))).toBe(false);
});

test("isShellControl does NOT drop a snapshot whose docJson embeds op-shell/", () => {
  // Regression: a legitimate snapshot-result carrying "op-shell/" inside its
  // docJson must reach the session — a substring check would drop it and hang
  // the awaiting save/backup.
  const msg = JSON.stringify({
    type: "op-bridge/snapshot-result",
    requestId: "r1",
    docJson: '{"text":"see op-shell/ready docs"}',
    generation: 2,
    revision: 1,
  });
  expect(msg).toContain("op-shell/");
  expect(isShellControl(msg)).toBe(false);
});

test("isShellControl rejects non-string / non-JSON / typeless payloads", () => {
  expect(isShellControl(123)).toBe(false);
  expect(isShellControl("not json")).toBe(false);
  expect(isShellControl(JSON.stringify({ origin: "x" }))).toBe(false);
  expect(isShellControl(JSON.stringify({ type: 42 }))).toBe(false);
});

test("parseShellReadyOrigin extracts the origin from a ready message", () => {
  expect(parseShellReadyOrigin(JSON.stringify({ type: "op-shell/ready", origin: "vscode-webview://x" }))).toBe(
    "vscode-webview://x",
  );
  expect(parseShellReadyOrigin(JSON.stringify({ type: "op-shell/ready" }))).toBeUndefined();
  expect(parseShellReadyOrigin(JSON.stringify({ type: "op-bridge/opened", generation: 1 }))).toBeUndefined();
  expect(parseShellReadyOrigin("not json")).toBeUndefined();
});
