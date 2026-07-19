import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { bundledDaemonEnv, pickRestartSource, resolveDaemonBinary } from "./restart-source";

test("dirty editor reopens durable bytes", () => {
  const r = pickRestartSource({ source: "save", json: '{"d":1}' }, true);
  expect(r).toEqual({ from: "durable", json: '{"d":1}' });
});

test("clean editor after save reopens from disk", () => {
  const r = pickRestartSource({ source: "save", json: "{}" }, false);
  expect(r).toEqual({ from: "disk" });
});

test("clean editor after revert reopens from disk", () => {
  const r = pickRestartSource({ source: "revert", json: "{}" }, false);
  expect(r).toEqual({ from: "disk" });
});

test("hot-exit backup (clean, source=backup) reopens durable, not disk", () => {
  // The recovered content differs from disk even though Rust is clean — reading
  // disk here would drop the recovery.
  const r = pickRestartSource({ source: "backup", json: '{"recovered":1}' }, false);
  expect(r).toEqual({ from: "durable", json: '{"recovered":1}' });
});

test("resolveDaemonBinary: explicit setting wins", () => {
  expect(resolveDaemonBinary("/opt/op-host-web-server", "/ws")).toBe("/opt/op-host-web-server");
  expect(resolveDaemonBinary("  /opt/x  ", "/ws")).toBe("/opt/x");
});

test("resolveDaemonBinary: falls back to workspace target/debug", () => {
  expect(resolveDaemonBinary("", "/ws")).toBe("/ws/target/debug/op-host-web-server");
  expect(resolveDaemonBinary(undefined, "/ws")).toBe("/ws/target/debug/op-host-web-server");
});

test("resolveDaemonBinary: null when nothing available", () => {
  expect(resolveDaemonBinary(undefined, undefined)).toBeNull();
  expect(resolveDaemonBinary("", undefined)).toBeNull();
});

describe("bundled runtime resolution", () => {
  const exe = process.platform === "win32" ? "op-host-web-server.exe" : "op-host-web-server";

  test("prefers the vsix-bundled binary over the workspace debug build", () => {
    const root = mkdtempSync(join(tmpdir(), "op-ext-"));
    try {
      mkdirSync(join(root, "bin"), { recursive: true });
      writeFileSync(join(root, "bin", exe), "");
      expect(resolveDaemonBinary(undefined, "/ws", root)).toBe(`${root}/bin/${exe}`);
      expect(resolveDaemonBinary(" /explicit ", "/ws", root)).toBe("/explicit");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("falls back to the workspace debug build when nothing is bundled", () => {
    expect(resolveDaemonBinary(undefined, "/ws", "/nonexistent-ext")).toBe(
      "/ws/target/debug/op-host-web-server",
    );
  });

  test("bundledDaemonEnv requires BOTH web asset dirs", () => {
    const root = mkdtempSync(join(tmpdir(), "op-ext-"));
    try {
      expect(bundledDaemonEnv(root)).toBeUndefined();
      mkdirSync(join(root, "web/pkg"), { recursive: true });
      expect(bundledDaemonEnv(root)).toBeUndefined();
      mkdirSync(join(root, "web/canvaskit"), { recursive: true });
      expect(bundledDaemonEnv(root)).toEqual({
        OPENPENCIL_WEB_BUNDLE_DIR: `${root}/web/pkg`,
        OPENPENCIL_CANVASKIT_DIR: `${root}/web/canvaskit`,
      });
      expect(bundledDaemonEnv(undefined)).toBeUndefined();
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
