import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockIPC } from "@tauri-apps/api/mocks";
import PersistentView from "./PersistentView";
import type { Repo } from "./Sidebar";
import type { SessionInfo } from "./TerminalTile";

const repo: Repo = {
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

function makeSession(id: string, overrides: Partial<SessionInfo> = {}): SessionInfo {
  const now = Math.floor(Date.now() / 1000);
  return {
    id,
    repoId: repo.id,
    repoName: repo.name,
    cwd: "/Users/me/Projects/alpha",
    command: ["claude", "--dangerously-skip-permissions"],
    startedAt: now,
    lastOutputAt: now,
    uptimeSecs: 0,
    frozen: false,
    exitCode: null,
    ...overrides,
  };
}

describe("PersistentView", () => {
  it("renders only the + tile and reports count 0 when no sessions", async () => {
    const onCount = vi.fn();
    render(<PersistentView repo={repo} onCountChange={onCount} />);

    expect(
      await screen.findByRole("button", { name: /open new terminal/i }),
    ).toBeInTheDocument();
    expect(screen.getByText(/^0 terminals$/)).toBeInTheDocument();
    await waitFor(() => expect(onCount).toHaveBeenLastCalledWith(0));
  });

  it("creates a terminal when + is clicked, increments the count, and adds a tile", async () => {
    const calls: Array<{ cmd: string; args: unknown }> = [];
    let nextId = 1;

    mockIPC(
      (cmd, args) => {
        calls.push({ cmd, args });
        switch (cmd) {
          case "pty_list":
            return [];
          case "inspect_local_repo":
            return {
              configuredBasePath: "~/Projects",
              repoPath: "/Users/me/Projects/alpha",
              exists: true,
              isGitRepo: true,
              currentBranch: "main",
              isClean: true,
              dirtyFiles: 0,
              error: null,
            };
          case "pty_create": {
            const id = `alpha-${1700000000 + nextId}-aabbccdd`;
            nextId++;
            return makeSession(id);
          }
          case "pty_write":
          case "pty_resize":
          case "pty_kill":
            return null;
          default:
            return undefined;
        }
      },
      { shouldMockEvents: true },
    );

    const onCount = vi.fn();
    const user = userEvent.setup();
    render(<PersistentView repo={repo} onCountChange={onCount} />);

    await user.click(
      await screen.findByRole("button", { name: /open new terminal/i }),
    );

    await waitFor(() => {
      expect(screen.getByText(/^1 terminal$/)).toBeInTheDocument();
    });
    expect(onCount).toHaveBeenLastCalledWith(1);

    const createCall = calls.find((c) => c.cmd === "pty_create");
    expect(createCall?.args).toMatchObject({
      args: {
        repoId: 1,
        repoName: "alpha",
        cwd: "/Users/me/Projects/alpha",
      },
    });
    // The tile renders its session id in the header.
    expect(screen.getByText(/alpha-\d+-[a-f0-9]{8}/)).toBeInTheDocument();
  });

  it("hydrates from pty_list on mount and shows existing tiles", async () => {
    const initial = [makeSession("alpha-1700000001-aaaaaaaa"), makeSession("alpha-1700000002-bbbbbbbb")];
    mockIPC(
      (cmd) => {
        if (cmd === "pty_list") return initial;
        return undefined;
      },
      { shouldMockEvents: true },
    );

    const onCount = vi.fn();
    render(<PersistentView repo={repo} onCountChange={onCount} />);

    expect(
      await screen.findByText("alpha-1700000001-aaaaaaaa"),
    ).toBeInTheDocument();
    expect(screen.getByText("alpha-1700000002-bbbbbbbb")).toBeInTheDocument();
    await waitFor(() => expect(onCount).toHaveBeenLastCalledWith(2));
  });

  it("removes the tile and calls pty_kill when close button is clicked", async () => {
    const calls: Array<{ cmd: string; args: unknown }> = [];
    const session = makeSession("alpha-1700000099-deadbeef");

    mockIPC(
      (cmd, args) => {
        calls.push({ cmd, args });
        if (cmd === "pty_list") return [session];
        return null;
      },
      { shouldMockEvents: true },
    );

    const onCount = vi.fn();
    const user = userEvent.setup();
    render(<PersistentView repo={repo} onCountChange={onCount} />);

    const closeBtn = await screen.findByRole("button", {
      name: /close terminal alpha-1700000099-deadbeef/i,
    });
    await user.click(closeBtn);

    await waitFor(() => {
      expect(screen.queryByText("alpha-1700000099-deadbeef")).not.toBeInTheDocument();
    });
    expect(calls.find((c) => c.cmd === "pty_kill")?.args).toMatchObject({
      args: { id: "alpha-1700000099-deadbeef" },
    });
    expect(onCount).toHaveBeenLastCalledWith(0);
  });

  it("surfaces an error when the local path does not exist and skips pty_create", async () => {
    let createCalls = 0;
    mockIPC(
      (cmd) => {
        if (cmd === "pty_list") return [];
        if (cmd === "inspect_local_repo") {
          return {
            configuredBasePath: "~/Projects",
            repoPath: "/Users/me/Projects/alpha",
            exists: false,
            isGitRepo: false,
            currentBranch: null,
            isClean: null,
            dirtyFiles: null,
            error: null,
          };
        }
        if (cmd === "pty_create") {
          createCalls++;
          return null;
        }
        return undefined;
      },
      { shouldMockEvents: true },
    );

    const user = userEvent.setup();
    render(<PersistentView repo={repo} />);
    await user.click(
      await screen.findByRole("button", { name: /open new terminal/i }),
    );

    expect(
      await screen.findByText(/local path not found/i),
    ).toBeInTheDocument();
    expect(createCalls).toBe(0);
  });
});
