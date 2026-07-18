// Pure helpers for `.fig` sources: path detection, the sibling `.op` save
// target, and byte→base64 encoding for the daemon's `/api/figma/convert`
// endpoint. Kept vscode-free so these decisions are unit-testable.

/** Case-insensitive `.fig` extension check. */
export function isFigPath(fsPath: string): boolean {
  return /\.fig$/i.test(fsPath);
}

/** `.fig` files never save back to themselves — every save (implicit or
 *  backup-driven) lands on the sibling `.op`, e.g. `/a/b/design.fig` →
 *  `/a/b/design.op`. */
export function figSaveTargetPath(figFsPath: string): string {
  return figFsPath.replace(/\.fig$/i, ".op");
}

/** Base64-encode raw `.fig` bytes for the JSON body of the convert request. */
export function encodeFigBytes(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64");
}
