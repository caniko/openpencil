// Pure helpers for the provider's daemon-crash recovery and binary resolution.
// Kept vscode-free so the recovery decisions (which bytes to reopen after a
// crash; where the daemon binary lives) are unit-testable.

export type DurableSource = "disk" | "save" | "revert" | "backup";
export interface LatestDurable {
  source: DurableSource;
  json: string;
}

/** After a daemon crash, decide which bytes to reopen with. If the editor was
 *  dirty, OR the durable state came from a hot-exit backup (Rust side is clean
 *  but its content differs from disk), reopen the durable bytes — reading disk
 *  would silently drop the recovered content. Otherwise the on-disk file is the
 *  source of truth. */
export function pickRestartSource(
  latest: LatestDurable,
  isDirty: boolean,
): { from: "durable"; json: string } | { from: "disk" } {
  if (isDirty || latest.source === "backup") {
    return { from: "durable", json: latest.json };
  }
  return { from: "disk" };
}

/** Resolve the op-host-web-server command prefix: an explicit setting wins,
 *  otherwise <workspaceRoot>/target/debug/op-host-web-server. Returns null when
 *  neither is available (the caller shows a guidance page). */
export function resolveDaemonBinary(
  configuredPath: string | undefined,
  workspaceRoot: string | undefined,
): string | null {
  const trimmed = configuredPath?.trim();
  if (trimmed) return trimmed;
  if (workspaceRoot) return `${workspaceRoot}/target/debug/op-host-web-server`;
  return null;
}
