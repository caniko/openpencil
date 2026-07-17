import { test, expect } from "bun:test";
import type { DaemonClient, DaemonLogger } from "./daemon-client";
import { DaemonPool } from "./daemon-pool";

// A stand-in DaemonClient: records dispose, lets tests drive onExit + fail modes.
class FakeClient {
  disposed = false;
  private exitCbs: Array<(code: number | null) => void> = [];
  constructor(public readonly id: number) {}
  onExit(cb: (code: number | null) => void): void {
    this.exitCbs.push(cb);
  }
  dispose(): Promise<void> {
    this.disposed = true;
    return Promise.resolve();
  }
  crash(): void {
    for (const cb of this.exitCbs) cb(1);
  }
  get baseUrl(): string {
    return `http://127.0.0.1:${this.id}`;
  }
}

function asClient(f: FakeClient): DaemonClient {
  return f as unknown as DaemonClient;
}

const silentLogger: DaemonLogger = { info: () => {}, error: () => {} };

function poolWithSpawns() {
  const spawned: FakeClient[] = [];
  let next = 1;
  let failNext = false;
  // When set, the NEXT spawn blocks on this gate (to simulate an in-flight
  // respawn); resolve it to complete the spawn.
  let gate: { promise: Promise<void>; release: () => void } | undefined;
  const spawn = async (): Promise<DaemonClient> => {
    if (failNext) {
      failNext = false;
      throw new Error("spawn failed");
    }
    const g = gate;
    gate = undefined;
    if (g) await g.promise;
    const f = new FakeClient(next++);
    spawned.push(f);
    return asClient(f);
  };
  const pool = new DaemonPool(spawn, silentLogger);
  return {
    pool,
    spawned,
    setFailNext: () => {
      failNext = true;
    },
    gateNextSpawn: () => {
      let release!: () => void;
      const promise = new Promise<void>((r) => (release = r));
      gate = { promise, release };
      return release;
    },
  };
}

test("acquire spawns once per file; second acquire reuses", async () => {
  const { pool, spawned } = poolWithSpawns();
  const a = await pool.acquire("/a.op", "vscode-webview://x");
  const b = await pool.acquire("/a.op", "vscode-webview://x");
  expect(spawned.length).toBe(1);
  expect(a).toBe(b);
});

test("concurrent acquires for the same file coalesce onto one spawn", async () => {
  const { pool, spawned } = poolWithSpawns();
  const [a, b] = await Promise.all([
    pool.acquire("/a.op", "o"),
    pool.acquire("/a.op", "o"),
  ]);
  expect(spawned.length).toBe(1);
  expect(a).toBe(b);
});

test("release disposes and evicts; re-acquire spawns fresh", async () => {
  const { pool, spawned } = poolWithSpawns();
  await pool.acquire("/a.op", "o");
  await pool.release("/a.op");
  expect((spawned[0] as unknown as FakeClient).disposed).toBe(true);
  await pool.acquire("/a.op", "o");
  expect(spawned.length).toBe(2);
});

test("setActive/active track the routing target; undefined clears", async () => {
  const { pool } = poolWithSpawns();
  let changes = 0;
  pool.onActiveChanged(() => {
    changes += 1;
  });
  const client = await pool.acquire("/a.op", "o");
  pool.setActive("/a.op");
  expect(pool.active?.filePath).toBe("/a.op");
  expect(pool.active?.client).toBe(client);
  expect(changes).toBe(1);
  pool.setActive(undefined);
  expect(pool.active).toBeUndefined();
  expect(changes).toBe(2);
});

test("setActive to a file with no live daemon is treated as clear", async () => {
  const { pool } = poolWithSpawns();
  pool.setActive("/never-spawned.op");
  expect(pool.active).toBeUndefined();
});

test("crash restarts once and notifies with the new client", async () => {
  const { pool, spawned } = poolWithSpawns();
  const restarts: Array<{ file: string; hasClient: boolean }> = [];
  pool.onRestart((file, client) => restarts.push({ file, hasClient: client !== undefined }));
  await pool.acquire("/a.op", "o");
  (spawned[0] as unknown as FakeClient).crash();
  await new Promise((r) => setTimeout(r, 0));
  expect(spawned.length).toBe(2); // respawned once
  expect(restarts).toEqual([{ file: "/a.op", hasClient: true }]);
  expect(pool.clientFor("/a.op")).toBe(asClient(spawned[1]));
});

test("second crash gives up, evicts, notifies with no client", async () => {
  const { pool, spawned } = poolWithSpawns();
  const restarts: Array<boolean> = [];
  pool.onRestart((_file, client) => restarts.push(client !== undefined));
  await pool.acquire("/a.op", "o");
  (spawned[0] as unknown as FakeClient).crash();
  await new Promise((r) => setTimeout(r, 0));
  (spawned[1] as unknown as FakeClient).crash();
  await new Promise((r) => setTimeout(r, 0));
  expect(spawned.length).toBe(2); // no third spawn
  expect(restarts).toEqual([true, false]);
  expect(pool.clientFor("/a.op")).toBeUndefined();
});

test("restart failure evicts and notifies with no client", async () => {
  const { pool, spawned, setFailNext } = poolWithSpawns();
  const restarts: Array<boolean> = [];
  pool.onRestart((_f, client) => restarts.push(client !== undefined));
  await pool.acquire("/a.op", "o");
  setFailNext();
  (spawned[0] as unknown as FakeClient).crash();
  await new Promise((r) => setTimeout(r, 0));
  expect(restarts).toEqual([false]);
  expect(pool.clientFor("/a.op")).toBeUndefined();
});

test("releasing during a respawn disposes the newly spawned daemon (no orphan)", async () => {
  const { pool, spawned, gateNextSpawn } = poolWithSpawns();
  const restarts: Array<boolean> = [];
  pool.onRestart((_f, client) => restarts.push(client !== undefined));
  await pool.acquire("/a.op", "o");
  // Crash A: handleCrash starts respawning, but the spawn is gated (in flight).
  const releaseSpawn = gateNextSpawn();
  (spawned[0] as unknown as FakeClient).crash();
  await new Promise((r) => setTimeout(r, 0)); // let handleCrash reach the await
  // User closes the panel now → release during the respawn window.
  await pool.release("/a.op");
  // Now the respawn completes.
  releaseSpawn();
  await new Promise((r) => setTimeout(r, 0));
  // A second daemon WAS spawned, but must have been disposed immediately.
  expect(spawned.length).toBe(2);
  expect((spawned[1] as unknown as FakeClient).disposed).toBe(true);
  expect(pool.clientFor("/a.op")).toBeUndefined(); // no lingering entry
  expect(restarts).toEqual([]); // no restart-with-client emitted for an owner-less daemon
});

test("dispose()-driven exit does not trigger a restart", async () => {
  const { pool, spawned } = poolWithSpawns();
  let restartCalls = 0;
  pool.onRestart(() => {
    restartCalls += 1;
  });
  await pool.acquire("/a.op", "o");
  await pool.release("/a.op"); // dispose → onExit fires but entry.disposed
  (spawned[0] as unknown as FakeClient).crash(); // late exit signal
  await new Promise((r) => setTimeout(r, 0));
  expect(restartCalls).toBe(0);
  expect(spawned.length).toBe(1);
});

test("disposeAll disposes every client and clears active", async () => {
  const { pool, spawned } = poolWithSpawns();
  await pool.acquire("/a.op", "o");
  await pool.acquire("/b.op", "o");
  pool.setActive("/a.op");
  await pool.disposeAll();
  expect(spawned.every((f) => (f as unknown as FakeClient).disposed)).toBe(true);
  expect(pool.active).toBeUndefined();
  expect(pool.clientFor("/a.op")).toBeUndefined();
});
