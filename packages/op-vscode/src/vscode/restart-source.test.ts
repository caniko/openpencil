import { test, expect } from "bun:test";
import { pickRestartSource, resolveDaemonBinary } from "./restart-source";

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
