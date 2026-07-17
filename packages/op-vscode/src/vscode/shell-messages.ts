// Pure classification of the shell's control messages (op-shell/*). Kept
// vscode-free and testable: a mis-classification here can drop a legitimate
// snapshot whose docJson happens to contain the text "op-shell/", hanging the
// save/backup that awaits it — so the match must be on the parsed top-level
// `type`, never a substring of the raw payload.

/** True only when `raw` is a JSON string whose top-level `type` is an
 *  op-shell/* control type. Business messages (op-bridge/*) and any payload
 *  that merely embeds "op-shell/" inside its data are NOT control traffic. */
export function isShellControl(raw: unknown): boolean {
  if (typeof raw !== "string") return false;
  try {
    const v = JSON.parse(raw) as { type?: unknown };
    return typeof v.type === "string" && v.type.startsWith("op-shell/");
  } catch {
    return false;
  }
}

/** The origin reported by an op-shell/ready control message, else undefined. */
export function parseShellReadyOrigin(raw: unknown): string | undefined {
  if (typeof raw !== "string") return undefined;
  try {
    const v = JSON.parse(raw) as { type?: unknown; origin?: unknown };
    if (v.type === "op-shell/ready" && typeof v.origin === "string") return v.origin;
  } catch {
    /* not a control message */
  }
  return undefined;
}
