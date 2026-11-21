import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { mockColorScheme } from "@/test/color-scheme";
import { blockStorage, fillStorage } from "@/test/storage";
import { STORAGE_KEY } from "./preferences";
import { ThemeProvider, useTheme } from "./theme-provider";

function Probe() {
  const { mode, update } = useTheme();
  return (
    <>
      <output>{mode}</output>
      <button type="button" onClick={() => update({ mode: "light" })}>
        light
      </button>
    </>
  );
}

const html = () => document.documentElement;

describe("ThemeProvider", () => {
  it("follows the system colour scheme live in system mode", () => {
    const scheme = mockColorScheme("light");
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    expect(html().dataset["mode"]).toBe("light");

    act(() => scheme.set("dark"));
    expect(html().dataset["mode"]).toBe("dark");
    expect(screen.getByRole("status")).toHaveTextContent("dark");

    act(() => scheme.set("light"));
    expect(html().dataset["mode"]).toBe("light");
  });

  it("stops following the system once a mode is chosen", async () => {
    const scheme = mockColorScheme("dark");
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    expect(html().dataset["mode"]).toBe("dark");

    await userEvent.click(screen.getByRole("button", { name: "light" }));
    act(() => scheme.set("dark"));
    expect(html().dataset["mode"]).toBe("light");
  });

  it("applies stored values and replaces invalid ones with defaults", () => {
    mockColorScheme("light");
    window.localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ palette: "soft", mode: "sepia", accentHue: 900, uiFont: "mono" }),
    );
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    expect(html().dataset).toMatchObject({ palette: "soft", mode: "light", uiFont: "mono" });
    expect(html().style.getPropertyValue("--accent-h")).toBe("250");
  });

  it("does not store anything until the user makes a choice", () => {
    mockColorScheme("light");
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("picks up a change made in another tab", () => {
    mockColorScheme("light");
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ palette: "warm" }));
    act(() => {
      window.dispatchEvent(new StorageEvent("storage", { key: STORAGE_KEY }));
    });
    expect(html().dataset["palette"]).toBe("warm");
  });

  it("runs on in-memory defaults when the browser refuses storage", async () => {
    mockColorScheme("dark");
    blockStorage();
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    expect(html().dataset).toMatchObject({ palette: "nothing", mode: "dark", uiFont: "default" });

    await userEvent.click(screen.getByRole("button", { name: "light" }));
    expect(html().dataset["mode"]).toBe("light");
  });

  it("keeps a choice for this visit when storage is full", async () => {
    mockColorScheme("dark");
    fillStorage();
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );

    await userEvent.click(screen.getByRole("button", { name: "light" }));
    expect(html().dataset["mode"]).toBe("light");
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("works where matchMedia does not exist", () => {
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    expect(html().dataset["mode"]).toBe("light");
  });
});
