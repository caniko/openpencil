// One managed daemon per open .op file. Concurrent acquires for the same file
// coalesce onto a single spawn. Tracks the routing "active" file (the McpProxy
// target) and applies the crash policy: a non-dispose exit respawns once, then
// evicts. No vscode import — the spawn factory is injected so it's unit-testable.

import type { DaemonClient, DaemonLogger } from "./daemon-client";

type SpawnFn = (filePath: string, allowOrigin: string) => Promise<DaemonClient>;

interface Entry {
  client: DaemonClient;
  allowOrigin: string;
  restarts: number;
  disposed: boolean;
}

export class DaemonPool {
  private readonly spawnFn: SpawnFn;
  private readonly logger: DaemonLogger;
  private readonly entries = new Map<string, Entry>();
  private readonly inflight = new Map<string, Promise<DaemonClient>>();
  private activeFile?: string;
  private readonly activeCbs: Array<() => void> = [];
  private readonly restartCbs: Array<(filePath: string, client: DaemonClient | undefined) => void> = [];

  constructor(spawnFn: SpawnFn, logger: DaemonLogger) {
    this.spawnFn = spawnFn;
    this.logger = logger;
  }

  /** Acquire the daemon for a file, spawning one if needed. Concurrent calls
   *  for the same file share a single spawn. */
  acquire(filePath: string, allowOrigin: string): Promise<DaemonClient> {
    const existing = this.entries.get(filePath);
    if (existing && !existing.disposed) return Promise.resolve(existing.client);
    const pending = this.inflight.get(filePath);
    if (pending) return pending;

    const p = this.spawnFn(filePath, allowOrigin).then((client) => {
      this.inflight.delete(filePath);
      const entry: Entry = { client, allowOrigin, restarts: 0, disposed: false };
      this.entries.set(filePath, entry);
      this.wireExit(filePath, entry);
      return client;
    });
    // On spawn failure, clear the inflight slot so a later acquire can retry.
    p.catch(() => this.inflight.delete(filePath));
    this.inflight.set(filePath, p);
    return p;
  }

  clientFor(filePath: string): DaemonClient | undefined {
    const e = this.entries.get(filePath);
    return e && !e.disposed ? e.client : undefined;
  }

  setActive(filePath: string | undefined): void {
    if (filePath !== undefined && !this.clientFor(filePath)) {
      // Cannot route to a file with no live daemon; treat as clear.
      filePath = undefined;
    }
    if (this.activeFile === filePath) return;
    this.activeFile = filePath;
    for (const cb of this.activeCbs) cb();
  }

  get active(): { filePath: string; client: DaemonClient } | undefined {
    if (this.activeFile === undefined) return undefined;
    const client = this.clientFor(this.activeFile);
    return client ? { filePath: this.activeFile, client } : undefined;
  }

  async release(filePath: string): Promise<void> {
    const entry = this.entries.get(filePath);
    if (!entry) return;
    entry.disposed = true;
    this.entries.delete(filePath);
    if (this.activeFile === filePath) this.setActive(undefined);
    await entry.client.dispose();
  }

  async disposeAll(): Promise<void> {
    const all = [...this.entries.values()];
    this.entries.clear();
    this.inflight.clear();
    this.activeFile = undefined;
    await Promise.all(
      all.map((e) => {
        e.disposed = true;
        return e.client.dispose();
      }),
    );
  }

  onActiveChanged(cb: () => void): void {
    this.activeCbs.push(cb);
  }
  onRestart(cb: (filePath: string, client: DaemonClient | undefined) => void): void {
    this.restartCbs.push(cb);
  }

  private wireExit(filePath: string, entry: Entry): void {
    entry.client.onExit(() => {
      // A dispose()-driven exit is expected (entry.disposed) — ignore it.
      if (entry.disposed) return;
      void this.handleCrash(filePath, entry);
    });
  }

  private async handleCrash(filePath: string, entry: Entry): Promise<void> {
    // Only the current entry for this file may drive a restart.
    if (this.entries.get(filePath) !== entry) return;
    if (entry.restarts >= 1) {
      // Second crash: give up, evict, notify with no client.
      this.logger.error(`daemon for ${filePath} crashed again; giving up`);
      this.entries.delete(filePath);
      if (this.activeFile === filePath) this.setActive(undefined);
      this.emitRestart(filePath, undefined);
      return;
    }
    this.logger.error(`daemon for ${filePath} exited; restarting once`);
    try {
      const client = await this.spawnFn(filePath, entry.allowOrigin);
      // The file may have been released (or the pool disposed) DURING the
      // respawn — both mark the crashed entry disposed. If so, the freshly
      // spawned daemon has no owner: dispose it immediately instead of adopting
      // it, otherwise it orphans (alive, no entry, no mount).
      if (entry.disposed || this.entries.get(filePath) !== entry) {
        await client.dispose();
        return;
      }
      const next: Entry = { client, allowOrigin: entry.allowOrigin, restarts: entry.restarts + 1, disposed: false };
      this.entries.set(filePath, next);
      this.wireExit(filePath, next);
      if (this.activeFile === filePath) {
        // Re-announce active so listeners recompute against the new client.
        for (const cb of this.activeCbs) cb();
      }
      this.emitRestart(filePath, client);
    } catch (err) {
      this.logger.error(`daemon for ${filePath} restart failed: ${String(err)}`);
      this.entries.delete(filePath);
      if (this.activeFile === filePath) this.setActive(undefined);
      this.emitRestart(filePath, undefined);
    }
  }

  private emitRestart(filePath: string, client: DaemonClient | undefined): void {
    for (const cb of this.restartCbs) cb(filePath, client);
  }
}
