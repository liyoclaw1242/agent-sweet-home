import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach } from "vitest";

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
      default:
        return undefined;
    }
  });
});

afterEach(() => {
  clearMocks();
  cleanup();
});
