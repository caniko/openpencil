import { test, expect } from "bun:test";
import { SessionRegistry } from "./session-registry";
import type { PenSession } from "./pen-session";

function fakeSession(id: string): PenSession {
  return { id } as unknown as PenSession;
}

test("register + setActive + activeSession returns the active one", () => {
  const r = new SessionRegistry();
  const a = fakeSession("a");
  const b = fakeSession("b");
  r.register("/a.op", a);
  r.register("/b.op", b);
  r.setActive("/a.op");
  expect(r.activeSession()).toBe(a);
  r.setActive("/b.op");
  expect(r.activeSession()).toBe(b);
});

test("setActive(undefined) clears the active pointer", () => {
  const r = new SessionRegistry();
  r.register("/a.op", fakeSession("a"));
  r.setActive("/a.op");
  r.setActive(undefined);
  expect(r.activeSession()).toBeUndefined();
});

test("setActive to an unregistered file is ignored", () => {
  const r = new SessionRegistry();
  r.register("/a.op", fakeSession("a"));
  r.setActive("/a.op");
  r.setActive("/never.op");
  // unchanged — still /a.op
  expect((r.activeSession() as unknown as { id: string }).id).toBe("a");
});

test("unregister clears active if it was the active file", () => {
  const r = new SessionRegistry();
  r.register("/a.op", fakeSession("a"));
  r.setActive("/a.op");
  r.unregister("/a.op");
  expect(r.activeSession()).toBeUndefined();
});

test("unregister a non-active file leaves active intact", () => {
  const r = new SessionRegistry();
  const a = fakeSession("a");
  r.register("/a.op", a);
  r.register("/b.op", fakeSession("b"));
  r.setActive("/a.op");
  r.unregister("/b.op");
  expect(r.activeSession()).toBe(a);
});
