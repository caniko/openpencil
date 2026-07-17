// filePath → PenSession registry. The active-session pointer's ONLY source of
// truth is the provider's view-state events (not the DaemonPool), so codegen /
// chat always target the editor the user is actually looking at. No vscode
// import — the provider drives it.

import type { PenSession } from "./pen-session";

export class SessionRegistry {
  private readonly sessions = new Map<string, PenSession>();
  private activeFile?: string;

  register(filePath: string, session: PenSession): void {
    this.sessions.set(filePath, session);
  }

  unregister(filePath: string): void {
    this.sessions.delete(filePath);
    if (this.activeFile === filePath) this.activeFile = undefined;
  }

  /** undefined clears the active pointer — the provider MUST call this when the
   *  user selects a non-OpenPencil editor, else the last .op stays the target. */
  setActive(filePath: string | undefined): void {
    if (filePath !== undefined && !this.sessions.has(filePath)) return;
    this.activeFile = filePath;
  }

  activeSession(): PenSession | undefined {
    return this.activeFile === undefined ? undefined : this.sessions.get(this.activeFile);
  }
}
