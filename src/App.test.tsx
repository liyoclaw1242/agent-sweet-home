import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockIPC } from "@tauri-apps/api/mocks";
import App from "./App";

interface MockRepo {
  id: number;
  name: string;
  fullName: string;
  description: string | null;
  htmlUrl: string;
  private: boolean;
  defaultBranch: string;
  stargazersCount: number;
  language: string | null;
  updatedAt: string;
}

function makeRepo(id: number, name: string, extra: Partial<MockRepo> = {}): MockRepo {
  return {
    id,
    name,
    fullName: `octocat/${name}`,
    description: null,
    htmlUrl: `https://example.com/${name}`,
    private: false,
    defaultBranch: "main",
    stargazersCount: 0,
    language: null,
    updatedAt: "2026-01-01T00:00:00Z",
    ...extra,
  };
}

describe("App", () => {
  it("renders the header and an empty sidebar", async () => {
    render(<App />);
    expect(
      screen.getByRole("heading", { level: 1, name: /Agent Sweet Home/i }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /open settings/i })).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByText(/no repositories yet/i)).toBeInTheDocument(),
    );
  });

  it("opens the settings dialog when clicking the gear", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: /open settings/i }));

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByLabelText(/GitHub Username/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/Auth Token/i)).toBeInTheDocument();
  });

  it("saves settings and reloads the repo list", async () => {
    const calls: Array<{ cmd: string; args: unknown }> = [];
    let savedUsername = "";

    mockIPC((cmd, args) => {
      calls.push({ cmd, args });
      switch (cmd) {
        case "get_settings":
          return { githubUsername: savedUsername, githubToken: "", localBasePath: "" };
        case "save_settings": {
          const a = args as { githubUsername: string };
          savedUsername = a.githubUsername;
          return null;
        }
        case "fetch_repos":
          return savedUsername ? [makeRepo(1, "demo", { language: "Rust" })] : [];
        default:
          return undefined;
      }
    });

    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: /open settings/i }));
    await user.type(screen.getByLabelText(/GitHub Username/i), "octocat");
    await user.type(screen.getByLabelText(/Auth Token/i), "ghp_xxx");
    await user.type(screen.getByLabelText(/Default Local Path/i), "~/Projects");
    await user.click(screen.getByRole("button", { name: /^save$/i }));

    expect(await screen.findByRole("button", { name: /demo/i })).toBeInTheDocument();
    expect(
      calls.find((c) => c.cmd === "save_settings")?.args,
    ).toMatchObject({
      githubUsername: "octocat",
      githubToken: "ghp_xxx",
      localBasePath: "~/Projects",
    });
  });

  it("shows an error in the sidebar when fetch_repos rejects", async () => {
    mockIPC((cmd) => {
      if (cmd === "get_settings") return { githubUsername: "", githubToken: "" };
      if (cmd === "fetch_repos") throw new Error("GitHub credentials not configured");
      return undefined;
    });

    render(<App />);
    expect(
      await screen.findByText(/GitHub credentials not configured/i),
    ).toBeInTheDocument();
  });

  it("renders all five tabs disabled until a repo is selected", async () => {
    render(<App />);
    const tabs = await screen.findAllByRole("tab");
    expect(tabs).toHaveLength(5);
    expect(tabs.map((t) => t.textContent)).toEqual([
      "Home",
      "Persistent(0)",
      "One-Shot(0)",
      "Graph",
      "Workflow(0)",
    ]);
    for (const tab of tabs) {
      expect(tab).toBeDisabled();
    }
  });

  it("keeps each repo's active tab independent across selections", async () => {
    mockIPC((cmd) => {
      if (cmd === "get_settings") return { githubUsername: "octocat", githubToken: "t" };
      if (cmd === "fetch_repos") return [makeRepo(1, "alpha"), makeRepo(2, "beta")];
      return undefined;
    });

    const user = userEvent.setup();
    render(<App />);

    const alphaButton = await screen.findByRole("button", { name: /alpha/i });
    const betaButton = screen.getByRole("button", { name: /beta/i });

    // Pick alpha → tabs unlock and Home is the default active tab.
    await user.click(alphaButton);
    const homeTab = screen.getByRole("tab", { name: /^Home$/i });
    expect(homeTab).not.toBeDisabled();
    expect(homeTab).toHaveAttribute("aria-selected", "true");

    // Switch alpha to Persistent.
    await user.click(screen.getByRole("tab", { name: /^Persistent/ }));
    expect(screen.getByRole("tab", { name: /^Persistent/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    // Switch to beta → it has its own state and starts on Home.
    await user.click(betaButton);
    expect(screen.getByRole("tab", { name: /^Home$/i })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    // Back to alpha → it remembers Persistent.
    await user.click(alphaButton);
    expect(screen.getByRole("tab", { name: /^Persistent/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });
});
