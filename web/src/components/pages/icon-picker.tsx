import type { FrimousseEmojiData } from "@blocknote/core/emoji-data";
import {
  useEffect,
  useId,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Loading } from "@/components/feedback/error-notice";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { characters, MAX_ICON } from "@/lib/validation";
import { strings } from "@/lib/strings";

const copy = strings.pages.icon;

/** Emoji in a row of the grid; the arrow keys move by it. */
const COLUMNS = 8;
/** The most emoji a search shows, the best matches first. */
const MATCHES = 96;

type IconPickerProps = {
  icon: string | null | undefined;
  onPick: (icon: string | null) => void;
  /** The button that opens it. */
  children: ReactNode;
};

/**
 * Picks a page's icon: an emoji from the built-in set, searched by name or
 * browsed by category, or a short text typed in. The emoji come with the app,
 * from BlockNote's own data, so nothing is fetched from elsewhere.
 */
export function IconPicker({ icon, onPick, children }: IconPickerProps) {
  const [open, setOpen] = useState(false);
  const pick = (next: string | null) => {
    setOpen(false);
    onPick(next);
  };
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent aria-label={copy.title}>
        {open && <IconChoices icon={icon} onPick={pick} />}
      </PopoverContent>
    </Popover>
  );
}

function IconChoices({
  icon,
  onPick,
}: {
  icon: string | null | undefined;
  onPick: IconPickerProps["onPick"];
}) {
  const searchId = useId();
  const textId = useId();
  const [data, setData] = useState<FrimousseEmojiData | null>(null);
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState(0);
  const [text, setText] = useState(icon ?? "");

  useEffect(() => {
    let current = true;
    const load = async () => {
      const { loadFrimousseData } = await import("@blocknote/core/emoji-data");
      const loaded = await loadFrimousseData("en");
      if (current) {
        setData(loaded);
      }
    };
    void load();
    return () => {
      current = false;
    };
  }, []);

  const textProblem =
    text.trim() !== "" && (characters(text.trim()) > MAX_ICON || /[\r\n]/.test(text))
      ? copy.textHint
      : undefined;

  const submitText = (event: FormEvent) => {
    event.preventDefault();
    const value = text.trim();
    if (value !== "" && textProblem === undefined) {
      onPick(value);
    }
  };

  return (
    <div className="grid gap-3">
      <div className="grid gap-1.5">
        <label htmlFor={searchId} className="text-sm font-medium">
          {copy.search}
        </label>
        <Input
          id={searchId}
          type="search"
          autoComplete="off"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
      </div>
      {data === null ? (
        <Loading label={copy.loading} />
      ) : (
        <EmojiGrid
          data={data}
          search={search}
          category={category}
          onCategory={setCategory}
          onPick={onPick}
        />
      )}
      <form onSubmit={submitText} noValidate className="grid gap-1.5">
        <label htmlFor={textId} className="text-sm font-medium">
          {copy.text}
        </label>
        <div className="flex gap-2">
          <Input
            id={textId}
            value={text}
            aria-describedby={`${textId}-hint`}
            {...(textProblem === undefined ? {} : { "aria-invalid": true })}
            onChange={(event) => setText(event.target.value)}
          />
          <Button
            type="submit"
            variant="outline"
            disabled={text.trim() === "" || textProblem !== undefined}
          >
            {copy.set}
          </Button>
        </div>
        <p id={`${textId}-hint`} className="text-xs text-muted-foreground">
          {copy.textHint}
        </p>
      </form>
      {icon && (
        <Button
          variant="ghost"
          size="sm"
          className="justify-self-start"
          onClick={() => onPick(null)}
        >
          {copy.remove}
        </Button>
      )}
    </div>
  );
}

type EmojiGridProps = {
  data: FrimousseEmojiData;
  search: string;
  category: number;
  onCategory: (index: number) => void;
  onPick: (icon: string) => void;
};

function EmojiGrid({ data, search, category, onCategory, onPick }: EmojiGridProps) {
  const needle = search.trim().toLowerCase();
  const shown =
    needle === ""
      ? data.emojis.filter((emoji) => emoji.category === category)
      : data.emojis
          .filter(
            (emoji) =>
              emoji.label.toLowerCase().includes(needle) ||
              emoji.tags.some((tag) => tag.toLowerCase().includes(needle)),
          )
          .slice(0, MATCHES);

  return (
    <div className="grid gap-2">
      {needle === "" && (
        <fieldset className="flex flex-wrap gap-1">
          <legend className="sr-only">{copy.categories}</legend>
          {data.categories.map((entry) => (
            <Button
              key={entry.index}
              variant={entry.index === category ? "default" : "outline"}
              size="sm"
              aria-pressed={entry.index === category}
              onClick={() => onCategory(entry.index)}
            >
              {entry.label}
            </Button>
          ))}
        </fieldset>
      )}
      {shown.length === 0 ? (
        <p className="text-sm text-muted-foreground">{copy.none}</p>
      ) : (
        <fieldset className="grid max-h-56 grid-cols-8 gap-0.5 overflow-y-auto">
          <legend className="sr-only">
            {needle === ""
              ? (data.categories.find((entry) => entry.index === category)?.label ??
                copy.categories)
              : copy.search}
          </legend>
          {shown.map((emoji, index) => (
            <button
              key={emoji.emoji}
              type="button"
              aria-label={emoji.label}
              tabIndex={index === 0 ? 0 : -1}
              onKeyDown={moveFocus}
              onClick={() => onPick(emoji.emoji)}
              className="grid aspect-square min-h-(--tap-sm) place-items-center rounded-md text-xl hover:bg-accent focus-visible:bg-accent"
            >
              {emoji.emoji}
            </button>
          ))}
        </fieldset>
      )}
    </div>
  );
}

/** The arrow keys, Home and End move through the grid, which is one tab stop. */
function moveFocus(event: KeyboardEvent<HTMLButtonElement>) {
  const buttons = [...(event.currentTarget.parentElement?.querySelectorAll("button") ?? [])];
  const at = buttons.indexOf(event.currentTarget);
  if (at === -1) {
    return;
  }
  const steps: Record<string, number> = {
    ArrowRight: 1,
    ArrowLeft: -1,
    ArrowDown: COLUMNS,
    ArrowUp: -COLUMNS,
    Home: -at,
    End: buttons.length - 1 - at,
  };
  const step = steps[event.key];
  if (step === undefined) {
    return;
  }
  event.preventDefault();
  const next = buttons[Math.min(buttons.length - 1, Math.max(0, at + step))];
  if (next !== undefined) {
    buttons[at]?.setAttribute("tabindex", "-1");
    next.setAttribute("tabindex", "0");
    next.focus();
  }
}
