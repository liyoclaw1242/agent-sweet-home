import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import OneShotView from "./OneShotView";
import type { Repo } from "./Sidebar";

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

interface RunInfo {
  id: string;
  repoId: number;
  repoName: string;
  cwd: string;
  prompt: string;
  argv: string[];
  status: "running" | "completed" | "failed" | "killed";
  startedAt: number;
  endedAt: number | null;
  exitCode: number | null;
  totalCostUsd: number | null;
  outputFormat: string;
}

function makeRun(id: string, overrides: Partial<RunInfo> = {}): RunInfo {
  return {
    id,
    repoId: 1,
    repoName: "alpha",
    cwd: "/Users/me/Projects/alpha",
    prompt: "do the thing",
    argv: ["claude", "-p", "--output-format", "stream-json", "--verbose", "do the thing"],
    status: "running",
    startedAt: 1700000000,
    endedAt: null,
    exitCode: null,
    totalCostUsd: null,
    outputFormat: "stream-json",
    ...overrides,
  };
}

describe("OneShotView", () => {
  it("hydrates the run list and reports the running count", async () => {
    const onCount = vi.fn();
    const runs = [
      makeRun("alpha-1-aa"),
      makeRun("alpha-2-bb", { status: "completed" }),
    ];
    mockIPC(
      (cmd) => {
        if (cmd === "one_shot_list") return runs;
        if (cmd === "one_shot_log") return [];
        return undefined;
      },
      { shouldMockEvents: true },
    );

    render(<OneShotView repo={repo} onCountChange={onCount} />);

    expect(await screen.findByText("alpha-1-aa")).toBeInTheDocument();
    expect(screen.getByText("alpha-2-bb")).toBeInTheDocument();
    // Running count is just the running rows, not all rows.
    await waitFor(() => expect(onCount).toHaveBeenLastCalledWith(1));
  });

  it("opens the modal, calls one_shot_start with built args, and shows the new run", async () => {
    const calls: Array<{ cmd: string; args: unknown }> = [];
    mockIPC(
      (cmd, args) => {
        calls.push({ cmd, args });
        switch (cmd) {
          case "one_shot_list":
            return [];
          case "one_shot_log":
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
          case "one_shot_start":
            return makeRun("alpha-100-newone", {
              prompt: (args as { args: { prompt: string } }).args.prompt,
            });
          default:
            return undefined;
        }
      },
      { shouldMockEvents: true },
    );

    const user = userEvent.setup();
    render(<OneShotView repo={repo} />);

    await user.click(
      await screen.findByRole("button", { name: /new one-shot run/i }),
    );

    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText(/^prompt$/i), "refactor auth");
    await user.click(within(dialog).getByRole("button", { name: /^start$/i }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    const startCall = calls.find((c) => c.cmd === "one_shot_start");
    expect(startCall?.args).toMatchObject({
      args: {
        repoId: 1,
        repoName: "alpha",
        cwd: "/Users/me/Projects/alpha",
        prompt: "refactor auth",
        outputFormat: "stream-json",
        verbose: true,
      },
    });

    // ID shows up twice — once in the list, once in the detail header — so
    // assert with findAllByText to disambiguate.
    const matches = await screen.findAllByText("alpha-100-newone");
    expect(matches.length).toBeGreaterThanOrEqual(1);
  });

  it("appends streamed lines from oneshot:line:{id} into the active run's log panel", async () => {
    const run = makeRun("alpha-77-ee");
    mockIPC(
      (cmd) => {
        if (cmd === "one_shot_list") return [run];
        if (cmd === "one_shot_log") return [];
        return undefined;
      },
      { shouldMockEvents: true },
    );

    const user = userEvent.setup();
    render(<OneShotView repo={repo} />);

    await user.click(await screen.findByRole("button", { name: /run alpha-77-ee/i }));

    // Once the listener is attached we can push a line in via the mock event
    // bus and expect the log panel to render it.
    await waitFor(() =>
      expect(screen.getByLabelText(/log output/i)).toBeInTheDocument(),
    );
    await emit(`oneshot:line:${run.id}`, {
      runId: run.id,
      seq: 0,
      ts: 1,
      stream: "stdout",
      text: '{"type":"system","subtype":"init"}',
    });

    expect(
      await screen.findByText(/"type":"system","subtype":"init"/),
    ).toBeInTheDocument();
  });

  it("blocks submission when prompt is empty without continue/resume", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "one_shot_list") return [];
        if (cmd === "one_shot_log") return [];
        return undefined;
      },
      { shouldMockEvents: true },
    );

    const user = userEvent.setup();
    render(<OneShotView repo={repo} />);
    await user.click(
      await screen.findByRole("button", { name: /new one-shot run/i }),
    );
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /^start$/i }));
    expect(within(dialog).getByText(/Prompt 不能空白/)).toBeInTheDocument();
  });
});
