import { cn } from "cn";
import { Check } from "lucide-react";
import { useState } from "react";
import { hueScope } from "@/components/nodes/hue";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { OPTION_HUES } from "@/lib/database-rules";
import { strings } from "@/lib/strings";

const copy = strings.databases.properties;

type HuePickerProps = {
  /** What the option is called, for the button's name. */
  name: string;
  hue: number | null | undefined;
  onPick: (hue: number | null) => void;
};

/**
 * The colour of an option: twelve hues around the wheel, drawn at the
 * palette's lightness and chroma so each reads in every palette and mode, or
 * grey. Each swatch says its colour's name.
 */
export function HuePicker({ name, hue, onPick }: HuePickerProps) {
  const [open, setOpen] = useState(false);
  const current = hue ?? null;
  const pick = (next: number | null) => {
    onPick(next);
    setOpen(false);
  };
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={copy.hue(name)}
          {...hueScope(current ?? undefined)}
        >
          <span
            aria-hidden
            className={cn(
              "size-3.5 rounded-full border",
              current === null ? "border-muted-foreground bg-muted" : "border-neon bg-neon",
            )}
          />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-auto" aria-label={copy.hue(name)}>
        <div className="grid grid-cols-7 gap-1">
          <Swatch label={copy.grey} selected={current === null} onPick={() => pick(null)} />
          {OPTION_HUES.map((entry) => (
            <Swatch
              key={entry}
              hue={entry}
              label={copy.hues[entry] ?? String(entry)}
              selected={current === entry}
              onPick={() => pick(entry)}
            />
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

type SwatchProps = { hue?: number; label: string; selected: boolean; onPick: () => void };

function Swatch({ hue, label, selected, onPick }: SwatchProps) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={selected}
      title={label}
      onClick={onPick}
      {...hueScope(hue)}
      className="grid size-(--tap-sm) place-items-center rounded-md hover:bg-accent"
    >
      <span
        aria-hidden
        className={cn(
          "grid size-5 place-items-center rounded-full border",
          hue === undefined
            ? "border-muted-foreground bg-muted text-foreground"
            : "border-neon bg-neon text-background",
        )}
      >
        {selected && <Check className="size-3" />}
      </span>
    </button>
  );
}
