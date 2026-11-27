import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { apiError } from "@/test/fixtures";
import { server } from "@/test/server";
import { ApiError } from "./errors";
import { UPLOADS_AT_ONCE, fileKindOf, fileUrl, uploadFile } from "./files";

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

  it("go a few at once, as the server takes them, and the rest wait their turn", async () => {
    let inFlight = 0;
    let most = 0;
    const answers: (() => void)[] = [];
    server.use(
      http.post("/api/v1/pages/:id/files", async () => {
        inFlight += 1;
        most = Math.max(most, inFlight);
        await new Promise<void>((resolve) => answers.push(resolve));
        inFlight -= 1;
        return HttpResponse.json(
          { id: FILE, url: fileUrl(FILE), page_id: PAGE, kind: "image" },
          { status: 201 },
        );
      }),
    );
    const files = Array.from(
      { length: UPLOADS_AT_ONCE + 3 },
      (_, n) => new File([new Uint8Array(1)], `${n}.png`, { type: "image/png" }),
    );
    const uploads = files.map((file) => uploadFile(PAGE, "image", file));
    // Answer each request as it arrives, until every upload has been sent.
    const answerAll = async (left: number): Promise<void> => {
      if (left === 0) {
        return;
      }
      await vi.waitFor(() => expect(answers.length).toBeGreaterThan(0));
      answers.shift()?.();
      return answerAll(left - 1);
    };
    await answerAll(files.length);
    await Promise.all(uploads);
    expect(most).toBe(UPLOADS_AT_ONCE);
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
