// Pure helpers for the provider's daemon-crash recovery and binary resolution.
// Kept vscode-free so the recovery decisions (which bytes to reopen after a
// crash; where the daemon binary lives) are unit-testable.

import { chmodSync, existsSync } from "node:fs";

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

/** Resolve the op-host-web-server command prefix. Order: an explicit setting
 *  wins; then a runtime bundled inside the installed extension (release vsix
 *  ships `bin/op-host-web-server[.exe]`); then the dev fallback
 *  <workspaceRoot>/target/debug/op-host-web-server. Returns null when none is
 *  available (the caller shows a guidance page). */
export function resolveDaemonBinary(
  configuredPath: string | undefined,
  workspaceRoot: string | undefined,
  extensionRoot?: string | undefined,
): string | null {
  const trimmed = configuredPath?.trim();
  if (trimmed) return trimmed;
  const bundled = bundledDaemonPath(extensionRoot);
  if (bundled) return bundled;
  if (workspaceRoot) return `${workspaceRoot}/target/debug/op-host-web-server`;
  return null;
}

/** The vsix-bundled daemon binary, when this install carries one. Artifact
 *  zips and some installers drop the unix exec bit, so restore it here —
 *  resolution time is the one place every spawn path passes through. */
export function bundledDaemonPath(extensionRoot: string | undefined): string | null {
  if (!extensionRoot) return null;
  const exe = process.platform === "win32" ? "op-host-web-server.exe" : "op-host-web-server";
  const candidate = `${extensionRoot}/bin/${exe}`;
  if (!existsSync(candidate)) return null;
  if (process.platform !== "win32") {
    try {
      chmodSync(candidate, 0o755);
    } catch {
      /* best-effort — spawn surfaces a real permission failure */
    }
  }
  return candidate;
}

/** Env overrides pointing the bundled daemon at the vsix-bundled web assets
 *  (wasm bundle + CanvasKit). Undefined when this install has no bundled web
 *  directory — the daemon then falls back to its own probing. */
export function bundledDaemonEnv(
  extensionRoot: string | undefined,
): Record<string, string> | undefined {
  if (!extensionRoot) return undefined;
  const pkg = `${extensionRoot}/web/pkg`;
  const canvaskit = `${extensionRoot}/web/canvaskit`;
  if (!existsSync(pkg) || !existsSync(canvaskit)) return undefined;
  return {
    OPENPENCIL_WEB_BUNDLE_DIR: pkg,
    OPENPENCIL_CANVASKIT_DIR: canvaskit,
  };
}
