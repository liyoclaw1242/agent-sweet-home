import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

export interface SessionInfo {
  id: string;
  repoId: number;
  repoName: string;
  cwd: string;
  command: string[];
  startedAt: number;
  lastOutputAt: number;
  uptimeSecs: number;
  frozen: boolean;
  exitCode: number | null;
}

interface Props {
  session: SessionInfo;
  onClose: (id: string) => void;
}

const FROZEN_AFTER_SECS = 15 * 60;

function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  if (m < 60) return `${m}m ${s.toString().padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  const mm = m % 60;
  return `${h}h ${mm.toString().padStart(2, "0")}m`;
}

function decodeBase64(payload: string): Uint8Array {
  const binary = atob(payload);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

function encodeBase64(data: string): string {
  // xterm.js delivers strings; round-trip through TextEncoder so we can
  // safely send unicode (emoji, CJK, etc.) over the IPC boundary.
  const bytes = new TextEncoder().encode(data);
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s);
}

export default function TerminalTile({ session, onClose }: Props) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const decoderRef = useRef<TextDecoder>(new TextDecoder());
  const [now, setNow] = useState<number>(Math.floor(Date.now() / 1000));
  const [lastOutputAt, setLastOutputAt] = useState<number>(session.lastOutputAt);
  const [exitCode, setExitCode] = useState<number | null>(session.exitCode);

  // Tick once per second so the uptime / frozen badge stays current.
  useEffect(() => {
    const id = window.setInterval(() => {
      setNow(Math.floor(Date.now() / 1000));
    }, 1000);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;

    const term = new Terminal({
      convertEol: false,
      cursorBlink: true,
      fontSize: 12,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, "DejaVu Sans Mono", monospace',
      theme: { background: "#000000" },
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(mount);
    try {
      fit.fit();
    } catch {
      // The mount node may not have layout in jsdom — safe to ignore.
    }

    let lastCols = term.cols;
    let lastRows = term.rows;

    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
      } catch {
        return;
      }
      if (term.cols !== lastCols || term.rows !== lastRows) {
        lastCols = term.cols;
        lastRows = term.rows;
        void invoke("pty_resize", {
          args: { id: session.id, cols: term.cols, rows: term.rows },
        }).catch(() => {});
      }
    });
    ro.observe(mount);

    const onDataDispose = term.onData((data: string) => {
      void invoke("pty_write", {
        args: { id: session.id, data: encodeBase64(data) },
      }).catch(() => {});
    });

    let unlistenOutput: UnlistenFn | null = null;
    let unlistenExit: UnlistenFn | null = null;
    let cancelled = false;

    void (async () => {
      const out = await listen<string>(`pty:output:${session.id}`, (event) => {
        const bytes = decodeBase64(event.payload);
        const text = decoderRef.current.decode(bytes, { stream: true });
        term.write(text);
        setLastOutputAt(Math.floor(Date.now() / 1000));
      });
      const ex = await listen<number>(`pty:exit:${session.id}`, (event) => {
        setExitCode(event.payload ?? 0);
      });
      if (cancelled) {
        out();
        ex();
      } else {
        unlistenOutput = out;
        unlistenExit = ex;
      }
    })();

    return () => {
      cancelled = true;
      onDataDispose.dispose();
      ro.disconnect();
      unlistenOutput?.();
      unlistenExit?.();
      term.dispose();
    };
  }, [session.id]);

  const uptime = Math.max(0, now - session.startedAt);
  const isFrozen =
    exitCode === null && now - lastOutputAt >= FROZEN_AFTER_SECS;

  const title = useMemo(
    () => `${session.id} — ${session.command.join(" ")} @ ${session.cwd}`,
    [session.id, session.command, session.cwd],
  );

  return (
    <section
      className="terminal-tile"
      aria-label={`Terminal ${session.id}`}
    >
      <header className="terminal-tile-header" title={title}>
        <span className="terminal-tile-id">{session.id}</span>
        <span className="terminal-tile-uptime" aria-label="uptime">
          {formatUptime(uptime)}
        </span>
        {isFrozen && (
          <span className="terminal-tile-frozen" aria-label="frozen">
            frozen
          </span>
        )}
        {exitCode !== null && (
          <span className="terminal-tile-exited" aria-label="exited">
            exited {exitCode}
          </span>
        )}
        <button
          type="button"
          className="terminal-tile-close"
          aria-label={`Close terminal ${session.id}`}
          onClick={() => onClose(session.id)}
        >
          ×
        </button>
      </header>
      <div className="terminal-tile-body">
        <div className="terminal-tile-mount" ref={mountRef} />
      </div>
    </section>
  );
}
