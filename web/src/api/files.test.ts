import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { apiError } from "@/test/fixtures";
import { server } from "@/test/server";
import { ApiError } from "./errors";
import { fileKindOf, fileUrl, uploadFile } from "./files";

const PAGE = "0193a5b2-0000-7000-8000-0000000000a3";
const FILE = "0193a5b2-0000-7000-8000-0000000000f1";

describe("files", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("are uploaded for the kind of block they go in, or else for what they are", () => {
    const png = { type: "image/png" };
    const pdf = { type: "application/pdf" };
    expect(fileKindOf("image", pdf)).toBe("image");
    expect(fileKindOf("video", png)).toBe("video");
    expect(fileKindOf("audio", png)).toBe("audio");
    expect(fileKindOf("file", png)).toBe("file");
    expect(fileKindOf("paragraph", png)).toBe("file");
    expect(fileKindOf(undefined, png)).toBe("image");
    expect(fileKindOf(undefined, { type: "video/mp4" })).toBe("video");
    expect(fileKindOf(undefined, { type: "audio/mpeg" })).toBe("audio");
    expect(fileKindOf(undefined, pdf)).toBe("file");
    expect(fileKindOf(undefined, { type: "" })).toBe("file");
  });

  it("go to the page as a form of one part, and come back with the path they are read at", async () => {
    const seen: { kind: string | null; name: string; size: number }[] = [];
    server.use(
      http.post("/api/v1/pages/:id/files", async ({ request, params }) => {
        const form = await request.formData();
        const file = form.get("file");
        seen.push({
          kind: new URL(request.url).searchParams.get("kind"),
          name: String(params["id"]),
          size: typeof file === "string" || file === null ? -1 : file.size,
        });
        return HttpResponse.json(
          { id: FILE, url: fileUrl(FILE), page_id: PAGE, kind: "image" },
          { status: 201 },
        );
      }),
    );
    // jsdom's files lose their names on their way into Node's requests, so the form is
    // asked what it was given.
    const append = vi.spyOn(FormData.prototype, "append");
    const file = new File([new Uint8Array(5)], "a.png", { type: "image/png" });
    const uploaded = await uploadFile(PAGE, "image", file);
    expect(uploaded.url).toBe(`/api/v1/files/${FILE}`);
    expect(seen).toEqual([{ kind: "image", name: PAGE, size: 5 }]);
    expect(append.mock.calls).toEqual([["file", file, "a.png"]]);
  });

  it("say why the server refused them", async () => {
    server.use(
      http.post("/api/v1/pages/:id/files", () =>
        apiError(415, "unsupported_type", "image files take PNG; this one is text/html"),
      ),
    );
    const refused = uploadFile(PAGE, "image", new File(["<html>"], "a.png"));
    await expect(refused).rejects.toBeInstanceOf(ApiError);
    await expect(refused).rejects.toMatchObject({ status: 415, code: "unsupported_type" });
  });
});
