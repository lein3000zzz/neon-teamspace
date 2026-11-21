import { describe, expect, it } from "vitest";
import { fnv1a32, hueFor } from "./hue";
import fixture from "./hue-vectors.json";

const encode = (text: string) => new TextEncoder().encode(text);

describe("hueFor", () => {
  it.each(fixture.vectors)("hashes $kind $id to hue $hue", ({ id, fnv1a32: hash, hue }) => {
    expect(hueFor(id)).toBe(hue);
    expect(hueFor(id)).toBe(hash % 360);
  });

  it("matches the published FNV-1a reference values", () => {
    expect(fnv1a32(encode(""))).toBe(0x811c9dc5);
    expect(fnv1a32(encode("a"))).toBe(0xe40c292c);
    expect(fnv1a32(encode("foobar"))).toBe(0xbf9cf968);
  });

  it("ignores UUID letter case", () => {
    const id = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
    expect(hueFor(id.toUpperCase())).toBe(hueFor(id));
  });

  it("spreads UUIDv7 ids that share a timestamp prefix", () => {
    const hues = new Set<number>();
    for (let i = 0; i < 200; i++) {
      const tail = i.toString(16).padStart(12, "0");
      hues.add(hueFor(`018f3c1e-7b2a-7c3d-9e4f-${tail}`));
    }
    expect(hues.size).toBeGreaterThan(100);
  });
});
