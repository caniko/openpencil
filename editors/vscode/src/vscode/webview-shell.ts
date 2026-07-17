// Webview relay shell: two pure HTML builders for the two-phase boot, plus the
// inline relay script. Phase 1 (boot) has no iframe — it reports the shell's
// real origin so the extension can spawn the daemon with the right
// --allow-origin. Phase 2 (full) embeds the daemon iframe and relays messages
// between the extension and the iframe with strict, origin-pinned forwarding.
//
// No vscode import — these are pure string functions, unit-tested directly.

/** Derive the origin ("scheme://host[:port]") from an absolute URL string. */
export function originOf(url: string): string {
  return new URL(url).origin;
}

/** Phase 1: no iframe. On load it reports window.origin so the extension can
 *  spawn the daemon with the correct --allow-origin, then waits for the
 *  extension to replace the HTML with the full shell. */
export function buildBootHtml(nonce: string): string {
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-${nonce}'; style-src 'unsafe-inline'">
<style>html,body{margin:0;height:100%;background:transparent}</style>
</head>
<body>
<script nonce="${nonce}">
(function () {
  const vscode = acquireVsCodeApi();
  // Report the shell's REAL document origin — asWebviewUri yields a resource
  // URI, not the origin, so it cannot be used to derive --allow-origin.
  vscode.postMessage(JSON.stringify({ type: "op-shell/ready", origin: window.origin }));
}());
</script>
</body>
</html>`;
}

/** Phase 2: full shell. Embeds the daemon iframe and relays messages both ways
 *  with strict source/origin checks. The payload is always a JSON string; the
 *  shell never parses business messages, only its own "op-shell/" control ones. */
export function buildWebviewHtml(opts: { iframeSrc: string; nonce: string }): string {
  const { iframeSrc, nonce } = opts;
  const iframeOrigin = originOf(iframeSrc);
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; frame-src ${iframeOrigin}; script-src 'nonce-${nonce}'; style-src 'unsafe-inline'">
<style>html,body{margin:0;height:100%;overflow:hidden;background:transparent}iframe{border:0;width:100%;height:100%;display:block}</style>
</head>
<body>
<iframe id="op-frame" src="${iframeSrc}" allow="clipboard-read; clipboard-write"></iframe>
<script nonce="${nonce}">
(function () {
  const vscode = acquireVsCodeApi();
  const frame = document.getElementById("op-frame");
  const IFRAME_ORIGIN = ${JSON.stringify(iframeOrigin)};

  // The extension calls webview.postMessage(jsonString). Report ready again so
  // the extension (which ignores duplicates) knows the full shell is live.
  vscode.postMessage(JSON.stringify({ type: "op-shell/ready", origin: window.origin }));

  window.addEventListener("message", function (e) {
    if (typeof e.data !== "string") return; // payloads are JSON strings only
    if (e.source === frame.contentWindow && e.origin === IFRAME_ORIGIN) {
      // page → shell → extension (acquireVsCodeApi is webview→extension only)
      vscode.postMessage(e.data);
    } else if (e.source !== frame.contentWindow) {
      // extension → shell → iframe. Control messages (op-shell/*) are handled
      // here; everything else is forwarded to the daemon page with an EXPLICIT
      // target origin (never "*").
      if (e.data.indexOf("op-shell/") !== -1) return;
      frame.contentWindow.postMessage(e.data, IFRAME_ORIGIN);
    }
  });
}());
</script>
</body>
</html>`;
}
