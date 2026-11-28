import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { bottomOf, useVisualViewport } from "./visual-viewport";

/** A visual viewport the test moves, as a keyboard or a pinch would. */
class FakeViewport extends EventTarget {
  offsetTop = 0;
  offsetLeft = 0;
  width = 390;
  height = 844;
  scale = 1;

  move(
    change: Partial<Pick<FakeViewport, "offsetTop" | "offsetLeft" | "width" | "height" | "scale">>,
  ) {
    Object.assign(this, change);
    this.dispatchEvent(new Event("resize"));
  }
}

/** A phone's screen, 390 by 844, its layout viewport the whole of it. */
function phone() {
  const viewport = new FakeViewport();
  vi.stubGlobal("visualViewport", viewport);
  vi.spyOn(document.documentElement, "clientHeight", "get").mockReturnValue(844);
  return viewport;
}

describe("the visual viewport", () => {
  it("follows the keyboard as it opens over the page and closes", () => {
    const viewport = phone();
    const { result } = renderHook(() => useVisualViewport());
    expect(result.current).toMatchObject({ height: 844, covered: 0 });

    // iOS lays the keyboard over the page: the visual viewport shrinks, the layout one stays.
    act(() => viewport.move({ height: 508 }));
    expect(result.current).toMatchObject({ top: 0, height: 508, covered: 336 });
    // Scrolled down inside the layout viewport, the keyboard still covers the rest of it.
    act(() => viewport.move({ offsetTop: 120 }));
    expect(result.current).toMatchObject({ top: 120, height: 508, covered: 216 });

    act(() => viewport.move({ offsetTop: 0, height: 844 }));
    expect(result.current).toMatchObject({ top: 0, height: 844, covered: 0 });
  });

  it("covers nothing where the page shrinks for the keyboard, as Android's can", () => {
    const viewport = phone();
    const { result } = renderHook(() => useVisualViewport());
    vi.spyOn(document.documentElement, "clientHeight", "get").mockReturnValue(508);

    act(() => viewport.move({ height: 508 }));
    expect(result.current).toMatchObject({ height: 508, covered: 0 });
  });

  it("keeps the same box while nothing moved, so nothing renders again", () => {
    const viewport = phone();
    const { result } = renderHook(() => useVisualViewport());
    const first = result.current;
    act(() => viewport.move({}));
    expect(result.current).toBe(first);
  });

  it("is the window's whole height without the API", () => {
    vi.stubGlobal("visualViewport", undefined);
    const { result } = renderHook(() => useVisualViewport());
    expect(result.current).toMatchObject({
      top: 0,
      left: 0,
      width: window.innerWidth,
      height: window.innerHeight,
      scale: 1,
      covered: 0,
    });
  });
});

describe("a bar along the visual viewport's bottom", () => {
  it("sits on the keyboard, as wide as the screen", () => {
    expect(bottomOf({ top: 0, left: 0, width: 390, height: 508, scale: 1, covered: 336 })).toEqual({
      width: "390px",
      transform: "translate(0px, 508px) translateY(-100%) scale(1)",
      transformOrigin: "0 100%",
    });
  });

  it("keeps its own size in a page zoomed in, along the part of it in sight", () => {
    // Zoomed in twice over, the screen shows a 195 by 254 part of the page, from 100, 300.
    expect(
      bottomOf({ top: 300, left: 100, width: 195, height: 254, scale: 2, covered: 0 }),
    ).toEqual({
      width: "390px",
      transform: "translate(100px, 554px) translateY(-100%) scale(0.5)",
      transformOrigin: "0 100%",
    });
  });
});
