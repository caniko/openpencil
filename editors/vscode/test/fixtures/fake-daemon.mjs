#!/usr/bin/env node
// Test double for `op-host-web-server --serve-web --managed`. It mimics only
// the parts DaemonClient depends on: a single-line handshake on stdout, a
// stdin-EOF self-kill lease, and a set of argv-selected failure modes.
//
// Modes (argv switches, checked before the real daemon flags):
//   --fake-port <n>      port echoed in the handshake (default 41234)
//   --fake-token <hex>   token echoed in the handshake (default "deadbeef")
//   --fake-version <v>   version echoed in the handshake (default "9.9.9")
//   --no-handshake       never print a handshake (stays alive → timeout path)
//   --garbage-handshake  print a non-JSON line
//   --delay <ms>         wait <ms> before printing the handshake
//   --early-exit         exit 1 immediately, before any handshake
//   --close-stdout       write half a handshake line, end stdout, stay alive
//   --echo-log           after the handshake, print a stderr line containing
//                        the token (redaction test)
//   --echo-argv          after the handshake, print the full received argv to
//                        stderr (arg-forwarding test; contains no token)
//
// The real daemon flags (--serve-web --managed --port 0 --file ...
// --allow-origin ...) are captured and echoed back inside the handshake's
// `argv` field so tests can assert they were forwarded verbatim.

import { closeSync } from "node:fs";

const argv = process.argv.slice(2);

function flag(name, fallback) {
  const i = argv.indexOf(name);
  return i >= 0 && i + 1 < argv.length ? argv[i + 1] : fallback;
}
function has(name) {
  return argv.includes(name);
}

const port = Number(flag("--fake-port", "41234"));
const token = flag("--fake-token", "deadbeef");
const version = flag("--fake-version", "9.9.9");

// Keep the process alive until stdin closes (the parent-death lease) unless a
// failure mode exits earlier. Reading stdin also lets `stdin.end()` reach us.
process.stdin.resume();
process.stdin.on("end", () => process.exit(0));
process.stdin.on("close", () => process.exit(0));

if (has("--early-exit")) {
  process.exit(1);
}

if (has("--close-stdout")) {
  // Half a line, then close fd 1 directly while staying alive: the client's
  // handshake reader sees the stream end before a full line arrives.
  // (process.stdout.end() does not reliably close the underlying fd, so we
  // close it explicitly after flushing the partial write.)
  process.stdout.write('{"ok":true,"por', () => {
    closeSync(1);
  });
  // Do not exit — this proves the client still cleans up the child.
  setInterval(() => {}, 1 << 30);
} else if (has("--no-handshake")) {
  // Silence — the client must hit its bounded handshake timeout.
  setInterval(() => {}, 1 << 30);
} else {
  const delay = Number(flag("--delay", "0"));
  const emit = () => {
    if (has("--garbage-handshake")) {
      process.stdout.write("this is not json at all\n");
    } else {
      const handshake = {
        ok: true,
        port,
        token,
        version,
        // Not part of the real contract — a test hook to assert forwarded args.
        argv,
      };
      process.stdout.write(JSON.stringify(handshake) + "\n");
      if (has("--echo-log")) {
        // A diagnostic line that leaks the token — the client must redact it
        // before handing it to the logger.
        process.stderr.write(`serving with token ${token} ready\n`);
      }
      if (has("--echo-argv")) {
        // Echo the daemon flags the client constructed so a test can assert
        // they were forwarded verbatim. Contains no token.
        process.stderr.write(`argv ${argv.join(" ")}\n`);
      }
    }
  };
  if (delay > 0) setTimeout(emit, delay);
  else emit();
}
