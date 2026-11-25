import { Type } from "lucide-react";
import { useId } from "react";
import type { Font, PageStyle, StyleChange } from "@/api/pages";
import { CheckboxField } from "@/components/forms/field";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { strings } from "@/lib/strings";

const copy = strings.pages.style;
const FONTS = ["default", "serif", "mono"] as const satisfies readonly Font[];
const FACES = { default: "font-default", serif: "font-serif", mono: "font-mono" } as const;

type StyleMenuProps = {
  style: PageStyle;
  disabled: boolean;
  /** Whether the page's width is the reader's to set: a database's views always take the full width. */
  widthChoice?: boolean;
  onChange: (change: StyleChange) => void;
};

/** How the page is set, as in Notion: its font, small text, and full width. */
export function StyleMenu({ style, disabled, widthChoice = true, onChange }: StyleMenuProps) {
  const fontId = useId();
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={copy.menu}
          className="text-muted-foreground"
        >
          <Type aria-hidden />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        aria-label={copy.menu}
        className="w-[min(18rem,calc(100vw-1rem))]"
      >
        <p id={fontId} className="text-sm font-medium">
          {copy.font}
        </p>
        <ToggleGroup
          type="single"
          variant="cards"
          aria-labelledby={fontId}
          value={style.font}
          disabled={disabled}
          onValueChange={(font) => {
            if (font !== "") {
              onChange({ font: font as Font });
            }
          }}
        >
          {FONTS.map((font) => (
            <ToggleGroupItem key={font} value={font} className="min-w-16 items-center">
              <span aria-hidden className={`text-xl ${FACES[font]}`}>
                Ag
              </span>
              <span className="text-xs">{copy.fonts[font]}</span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <CheckboxField
          label={copy.smallText}
          checked={style.small_text}
          disabled={disabled}
          onCheckedChange={(small_text) => onChange({ small_text })}
        />
        {widthChoice && (
          <CheckboxField
            label={copy.fullWidth}
            checked={style.full_width}
            disabled={disabled}
            onCheckedChange={(full_width) => onChange({ full_width })}
          />
        )}
      </PopoverContent>
    </Popover>
  );
}
