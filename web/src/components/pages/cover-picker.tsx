import { cn } from "cn";
import { Upload } from "lucide-react";
import { useRef, useState, type ReactNode } from "react";
import { fileUrl } from "@/api/files";
import type { Cover } from "@/api/pages";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { coverImage, COVERS } from "@/lib/covers";
import { strings } from "@/lib/strings";

const copy = strings.pages.cover;

/** The images a cover takes, as the server takes them for an image. */
export const COVER_TYPES = "image/png,image/jpeg,image/gif,image/webp,image/avif,image/svg+xml";

type CoverPickerProps = {
  cover: Cover | null | undefined;
  /** Whether the page has a cover of either kind, which the picker offers to remove. */
  covered: boolean;
  onPick: (cover: Cover | null) => void;
  /** Takes an image the user chose, to upload as the cover. */
  onUpload: (file: File) => void;
  children: ReactNode;
};

/**
 * Picks a page's cover from the presets, each drawn as it will look on this
 * theme, or uploads an image of the user's as one.
 */
export function CoverPicker({ cover, covered, onPick, onUpload, children }: CoverPickerProps) {
  const [open, setOpen] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const pick = (next: Cover | null) => {
    setOpen(false);
    onPick(next);
  };
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent aria-label={copy.title}>
        <fieldset className="grid gap-3">
          <legend className="mb-1 text-sm font-medium">{copy.title}</legend>
          <p className="text-xs text-muted-foreground">{copy.lede}</p>
          <div className="grid grid-cols-4 gap-2 sm:grid-cols-5">
            {COVERS.map((preset) => (
              <button
                key={preset}
                type="button"
                aria-label={copy.names[preset]}
                aria-pressed={preset === cover}
                onClick={() => pick(preset)}
                style={{ backgroundImage: coverImage(preset) }}
                className={cn(
                  "h-(--tap) rounded-md border border-border",
                  preset === cover && "outline-2 outline-offset-2 outline-primary",
                )}
              />
            ))}
          </div>
        </fieldset>
        <div className="flex flex-wrap gap-1">
          <Button variant="outline" size="sm" onClick={() => input.current?.click()}>
            <Upload aria-hidden />
            {copy.upload}
          </Button>
          <input
            ref={input}
            type="file"
            accept={COVER_TYPES}
            aria-label={copy.uploadLabel}
            className="sr-only"
            tabIndex={-1}
            onChange={(event) => {
              const file = event.target.files?.[0];
              event.target.value = "";
              if (file !== undefined) {
                setOpen(false);
                onUpload(file);
              }
            }}
          />
          {covered && (
            <Button variant="ghost" size="sm" onClick={() => pick(null)}>
              {copy.remove}
            </Button>
          )}
        </div>
        <p className="text-xs text-muted-foreground">{copy.uploadHint}</p>
      </PopoverContent>
    </Popover>
  );
}

type CoverBandProps = {
  cover: Cover | null | undefined;
  /** The id of an image of the page shown as its cover, in place of a preset. */
  image: string | null | undefined;
  children?: ReactNode;
};

/** A page's cover as a band across the top of the page: its image, or a preset. */
export function CoverBand({ cover, image, children }: CoverBandProps) {
  const background =
    image !== null && image !== undefined
      ? `url("${fileUrl(image)}")`
      : cover !== null && cover !== undefined
        ? coverImage(cover)
        : undefined;
  return (
    <div
      {...(image !== null && image !== undefined
        ? { "data-cover-image": image }
        : { "data-cover": cover ?? undefined })}
      style={background === undefined ? {} : { backgroundImage: background }}
      className="relative flex h-32 items-end justify-end rounded-lg bg-cover bg-center p-2 sm:h-44"
    >
      {children}
    </div>
  );
}
