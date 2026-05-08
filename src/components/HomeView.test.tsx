import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { mockIPC } from "@tauri-apps/api/mocks";
import HomeView from "./HomeView";
import type { Repo } from "./Sidebar";

const repoFixture: Repo = {
  id: 1,
  name: "alpha",
  fullName: "octocat/alpha",
  description: null,
  htmlUrl: "https://example.com/alpha",
  private: false,
  defaultBranch: "main",
  stargazersCount: 0,
  language: null,
  updatedAt: "2026-01-01T00:00:00Z",
};

const REFRESH_MS = 15 * 60 * 1000;

const inspectionFixture = {
  configuredBasePath: "",
  repoPath: "",
  exists: false,
  isGitRepo: false,
  currentBranch: null,
  isClean: null,
  dirtyFiles: null,
  error: null,
};

describe("HomeView — periodic refresh + cleanup", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("auto-refreshes issues + PRs every 15 minutes and stops on unmount", async () => {
    let inspectCalls = 0;
    let issueCalls = 0;
    let prCalls = 0;

    mockIPC((cmd) => {
      switch (cmd) {
        case "inspect_local_repo":
          inspectCalls++;
          return inspectionFixture;
        case "fetch_issues":
          issueCalls++;
          return [];
        case "fetch_prs":
          prCalls++;
          return [];
        default:
          return undefined;
      }
    });

    const { unmount } = render(<HomeView repo={repoFixture} />);

    // Mount fires the initial pre-check + first GitHub refresh. Flushing
    // pending microtasks lets those promises resolve before we assert.
    await vi.advanceTimersByTimeAsync(0);
    expect(inspectCalls).toBe(1);
    expect(issueCalls).toBe(1);
    expect(prCalls).toBe(1);

    // 15 min later: another refresh of issues + PRs (no new inspection).
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(inspectCalls).toBe(1);
    expect(issueCalls).toBe(2);
    expect(prCalls).toBe(2);

    // Unmount → effect cleanup clears the interval. Advancing time
    // further must NOT trigger more invocations.
    unmount();
    await vi.advanceTimersByTimeAsync(REFRESH_MS * 3);
    expect(inspectCalls).toBe(1);
    expect(issueCalls).toBe(2);
    expect(prCalls).toBe(2);
  });
});

describe("HomeView — rendering", () => {
  it("renders fetched issues with #number, title and label", async () => {
    mockIPC((cmd) => {
      if (cmd === "inspect_local_repo") return inspectionFixture;
      if (cmd === "fetch_issues") {
        return [
          {
            id: 11,
            number: 42,
            title: "Fix the thing",
            htmlUrl: "https://example.com/issues/42",
            labels: [{ name: "bug", color: "d73a4a" }],
          },
        ];
      }
      if (cmd === "fetch_prs") return [];
      return undefined;
    });

    render(<HomeView repo={repoFixture} />);

    expect(await screen.findByText("#42")).toBeInTheDocument();
    expect(screen.getByText("Fix the thing")).toBeInTheDocument();
    expect(screen.getByText("bug")).toBeInTheDocument();
  });

  it("renders fetched PRs with #number, title and draft badge", async () => {
    mockIPC((cmd) => {
      if (cmd === "inspect_local_repo") return inspectionFixture;
      if (cmd === "fetch_issues") return [];
      if (cmd === "fetch_prs") {
        return [
          {
            id: 7,
            number: 7,
            title: "Add tabs",
            htmlUrl: "https://example.com/pulls/7",
            draft: true,
          },
        ];
      }
      return undefined;
    });

    render(<HomeView repo={repoFixture} />);

    expect(await screen.findByText("#7")).toBeInTheDocument();
    expect(screen.getByText("Add tabs")).toBeInTheDocument();
    expect(screen.getByText("draft")).toBeInTheDocument();
  });

  it("shows a 'missing' badge when the local path does not exist", async () => {
    mockIPC((cmd) => {
      if (cmd === "inspect_local_repo") {
        return {
          ...inspectionFixture,
          configuredBasePath: "~/Projects",
          repoPath: "/Users/me/Projects/alpha",
          exists: false,
        };
      }
      if (cmd === "fetch_issues") return [];
      if (cmd === "fetch_prs") return [];
      return undefined;
    });

    render(<HomeView repo={repoFixture} />);

    expect(await screen.findByText(/\/Users\/me\/Projects\/alpha/i)).toBeInTheDocument();
    expect(screen.getByText("missing")).toBeInTheDocument();
  });
});
