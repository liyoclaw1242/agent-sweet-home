import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, vi } from "vitest";

// xterm.js wants real canvas + ResizeObserver. We don't unit-test the rendering
// pipeline, so swap in lightweight mocks for the modules our TerminalTile
// imports. Tests that care about terminal lifecycle assert on Tauri IPC and
// event listeners, not on the xterm instance.
vi.mock("@xterm/xterm", () => {
  class FakeTerminal {
    cols = 80;
    rows = 24;
    private dataHandlers: Array<(d: string) => void> = [];
    open = vi.fn();
    write = vi.fn();
    dispose = vi.fn();
    focus = vi.fn();
    clear = vi.fn();
    loadAddon = vi.fn();
    resize = vi.fn((cols: number, rows: number) => {
      this.cols = cols;
      this.rows = rows;
    });
    onData(handler: (d: string) => void) {
      this.dataHandlers.push(handler);
      return { dispose: () => {} };
    }
    __emitData(d: string) {
      for (const h of this.dataHandlers) h(d);
    }
  }
  return { Terminal: FakeTerminal };
});

vi.mock("@xterm/addon-fit", () => {
  class FakeFitAddon {
    activate = vi.fn();
    dispose = vi.fn();
    fit = vi.fn();
    proposeDimensions = vi.fn(() => ({ cols: 80, rows: 24 }));
  }
  return { FitAddon: FakeFitAddon };
});

if (typeof globalThis.ResizeObserver === "undefined") {
  class FakeResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver =
    FakeResizeObserver as unknown as typeof ResizeObserver;
}

beforeEach(() => {
  // Default IPC mock — keeps every screen happy in tests that don't
  // care about the backend. Override per test with mockIPC() when needed.
  mockIPC((cmd) => {
    switch (cmd) {
      case "get_settings":
        return { githubUsername: "", githubToken: "", localBasePath: "" };
      case "save_settings":
        return null;
      case "fetch_repos":
        return [];
      case "fetch_issues":
        return [];
      case "fetch_prs":
        return [];
      case "inspect_local_repo":
        return {
          configuredBasePath: "",
          repoPath: "",
          exists: false,
          isGitRepo: false,
          currentBranch: null,
          isClean: null,
          dirtyFiles: null,
          error: null,
        };
      case "pty_list":
        return [];
      case "pty_create":
      case "pty_write":
      case "pty_resize":
      case "pty_kill":
        return null;
      default:
        return undefined;
    }
  });
});

afterEach(() => {
  // Unmount components first so their effect cleanups (e.g. event.unlisten)
  // run while the Tauri mock internals are still installed; then strip the
  // mocks so the next test starts clean.
  cleanup();
  clearMocks();
});
