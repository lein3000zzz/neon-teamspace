import { cn } from "cn";
import { useState, type ReactNode } from "react";
import type { Cover } from "@/api/pages";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { coverImage, COVERS } from "@/lib/covers";
import { strings } from "@/lib/strings";

const copy = strings.pages.cover;

type CoverPickerProps = {
  cover: Cover | null | undefined;
  onPick: (cover: Cover | null) => void;
  children: ReactNode;
};

/** Picks a page's cover from the presets, each drawn as it will look on this theme. */
export function CoverPicker({ cover, onPick, children }: CoverPickerProps) {
  const [open, setOpen] = useState(false);
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
        {cover && (
          <Button
            variant="ghost"
            size="sm"
            className="justify-self-start"
            onClick={() => pick(null)}
          >
            {copy.remove}
          </Button>
        )}
      </PopoverContent>
    </Popover>
  );
}

/** A page's cover as a band across the top of the page. */
export function CoverBand({ cover, children }: { cover: Cover; children?: ReactNode }) {
  return (
    <div
      data-cover={cover}
      style={{ backgroundImage: coverImage(cover) }}
      className="relative flex h-32 items-end justify-end rounded-lg p-2 sm:h-44"
    >
      {children}
    </div>
  );
}
