import { useMutation } from "@tanstack/react-query";
import { call } from "./call";
import { api, type Schemas } from "./client";
import { useUpdatePage } from "./pages";

export type UploadedFile = Schemas["File"];
export type FileKind = Schemas["FileKind"];

/** How many uploads this tab runs at once: as many as the server runs of one person's. */
export const UPLOADS_AT_ONCE = 4;

let uploading = 0;
const waitingToUpload: (() => void)[] = [];

/** Runs `upload` once fewer than UPLOADS_AT_ONCE run, the others waiting their turn in order. */
async function inTurn<T>(upload: () => Promise<T>): Promise<T> {
  if (uploading >= UPLOADS_AT_ONCE) {
    await new Promise<void>((resolve) => waitingToUpload.push(resolve));
  } else {
    uploading += 1;
  }
  try {
    return await upload();
  } finally {
    // The turn goes to the next one waiting, or back.
    const next = waitingToUpload.shift();
    if (next === undefined) {
      uploading -= 1;
    } else {
      next();
    }
  }
}

/**
 * Uploads a file to a page, for one of its blocks of `kind`, or its cover, as
 * a form of one part. The server says what the file is from its bytes and
 * refuses a type the kind does not take, a file past the largest it takes,
 * and one the workspace has no room for. A few go at once, as the server runs
 * a few of one person's: dropping a folder of images queues the rest.
 */
export function uploadFile(pageId: string, kind: FileKind, file: File): Promise<UploadedFile> {
  return inTurn(() =>
    call(
      api.POST("/api/v1/pages/{id}/files", {
        params: { path: { id: pageId }, query: { kind } },
        // The schema writes the part as a string, as OpenAPI writes binary; the form carries the file.
        body: { file: file.name },
        bodySerializer: () => {
          const form = new FormData();
          form.append("file", file, file.name);
          return form;
        },
      }),
    ),
  );
}

/** The kinds of blocks that show a file, by their type's name. */
const KINDS: Record<string, FileKind> = { image: "image", video: "video", audio: "audio" };

/**
 * What a file is uploaded for: the kind of the block it goes in, or, with no
 * block, what its type says it is. Anything else is a file to download.
 */
export function fileKindOf(blockType: string | undefined, file: Pick<File, "type">): FileKind {
  if (blockType !== undefined) {
    return KINDS[blockType] ?? "file";
  }
  return KINDS[file.type.split("/")[0] ?? ""] ?? "file";
}

/** Where a file is read: the path the server serves it at, which documents keep. */
export function fileUrl(id: string): string {
  return `/api/v1/files/${id}`;
}

/** Uploads an image to a page and makes it the page's cover, in place of a preset. */
export function useUploadCover(workspaceId: string, pageId: string) {
  const update = useUpdatePage(workspaceId, pageId);
  return useMutation({
    mutationFn: async (file: File) => {
      const uploaded = await uploadFile(pageId, "image", file);
      return update.mutateAsync({ cover_image: uploaded.id });
    },
  });
}
