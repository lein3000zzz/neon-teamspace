import { Monitor, Moon, RotateCcw, Sun } from "lucide-react";
import { useCallback, useEffect, useId, useRef } from "react";
import { useSavePreferences } from "@/api/account";
import { errorText } from "@/components/feedback/error-notice";
import { SettingSection } from "@/components/settings/setting-section";
import { AccentPicker } from "@/components/theme/accent-picker";
import { PaletteSwatch } from "@/components/theme/palette-swatch";
import { ThemePreview } from "@/components/theme/theme-preview";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { strings } from "@/lib/strings";
import {
  DEFAULT_PREFERENCES,
  MODE_PREFERENCES,
  PALETTES,
  UI_FONTS,
  matchOption,
  type ModePreference,
  type ThemePreferences,
} from "@/theme/preferences";
import { useTheme } from "@/theme/theme-provider";

const MODE_ICONS: Record<ModePreference, typeof Monitor> = {
  system: Monitor,
  light: Sun,
  dark: Moon,
};

/** How long after the last change the look is saved, so a slider drag saves once. */
export const SAVE_DELAY_MS = 400;

/**
 * Saves the look to the account a moment after the last change, and at once
 * when the page closes with one still waiting.
 */
function useSaveToAccount() {
  const save = useSavePreferences();
  const { mutate } = save;
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const waiting = useRef<ThemePreferences | undefined>(undefined);

  const flush = useCallback(() => {
    clearTimeout(timer.current);
    timer.current = undefined;
    if (waiting.current !== undefined) {
      mutate(waiting.current);
      waiting.current = undefined;
    }
  }, [mutate]);

  const schedule = useCallback(
    (preferences: ThemePreferences) => {
      waiting.current = preferences;
      clearTimeout(timer.current);
      timer.current = setTimeout(flush, SAVE_DELAY_MS);
    },
    [flush],
  );

  useEffect(() => flush, [flush]);

  return { schedule, save };
}

/**
 * The accent: the hue of the workspace in view, by default, or one of the
 * user's own, on the slider and presets. Choosing their own starts from the
 * hue shown, so nothing changes until they move it.
 */
function AccentChoice({
  labelledBy,
  onChange,
}: {
  labelledBy: string;
  onChange: (accentHue: number | null) => void;
}) {
  const { preferences, accent } = useTheme();
  const sourceId = useId();
  const own = preferences.accentHue;
  const copy = strings.appearance;
  return (
    <div className="grid gap-4">
      <span id={sourceId} className="sr-only">
        {copy.accentSource}
      </span>
      <ToggleGroup
        type="single"
        aria-labelledby={sourceId}
        value={own === null ? "workspace" : "own"}
        onValueChange={(value) => {
          if (value === "workspace") {
            onChange(null);
          } else if (value === "own") {
            onChange(own ?? accent);
          }
        }}
      >
        <ToggleGroupItem value="workspace">{copy.accentWorkspace}</ToggleGroupItem>
        <ToggleGroupItem value="own">{copy.accentOwn}</ToggleGroupItem>
      </ToggleGroup>
      {own === null ? (
        <p className="text-sm text-muted-foreground">{copy.accentFollows(accent)}</p>
      ) : (
        <AccentPicker hue={own} labelledBy={labelledBy} onChange={onChange} />
      )}
    </div>
  );
}

/**
 * Palette, mode, accent and interface font. Every change applies at once,
 * stays on this device for the next first paint, and is saved to the account
 * so other browsers pick it up when someone signs in there.
 */
export function AppearancePage() {
  const { preferences, mode, systemMode, update, reset } = useTheme();
  const { schedule, save } = useSaveToAccount();
  const copy = strings.appearance;

  const change = (patch: Partial<ThemePreferences>) => {
    update(patch);
    schedule({ ...preferences, ...patch });
  };

  // A single-choice toggle group reports "" when its active item is pressed
  // again; matchOption drops that, so a choice can't be cleared.
  return (
    <div className="grid gap-8">
      <p className="max-w-[62ch] text-muted-foreground">{copy.lede}</p>
      <ThemePreview />

      <SettingSection title={copy.palette} hint={copy.paletteHint}>
        {(headingId) => (
          <ToggleGroup
            type="single"
            variant="cards"
            aria-labelledby={headingId}
            value={preferences.palette}
            onValueChange={(value) => {
              const palette = matchOption(PALETTES, value);
              if (palette) {
                change({ palette });
              }
            }}
            className="grid w-full grid-cols-[repeat(auto-fill,minmax(8.5rem,1fr))]"
          >
            {PALETTES.map((palette) => (
              <ToggleGroupItem key={palette} value={palette}>
                <PaletteSwatch palette={palette} mode={mode} />
                <span className="text-left">{copy.palettes[palette]}</span>
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        )}
      </SettingSection>

      <SettingSection title={copy.mode} hint={copy.modeHint(copy.modes[systemMode])}>
        {(headingId) => (
          <ToggleGroup
            type="single"
            aria-labelledby={headingId}
            value={preferences.mode}
            onValueChange={(value) => {
              const next = matchOption(MODE_PREFERENCES, value);
              if (next) {
                change({ mode: next });
              }
            }}
          >
            {MODE_PREFERENCES.map((option) => {
              const Icon = MODE_ICONS[option];
              return (
                <ToggleGroupItem key={option} value={option}>
                  <Icon aria-hidden />
                  {copy.modes[option]}
                </ToggleGroupItem>
              );
            })}
          </ToggleGroup>
        )}
      </SettingSection>

      <SettingSection title={copy.accent} hint={copy.accentHint}>
        {(headingId) => (
          <AccentChoice labelledBy={headingId} onChange={(accentHue) => change({ accentHue })} />
        )}
      </SettingSection>

      <SettingSection title={copy.uiFont} hint={copy.uiFontHint}>
        {(headingId) => (
          <ToggleGroup
            type="single"
            aria-labelledby={headingId}
            value={preferences.uiFont}
            onValueChange={(value) => {
              const uiFont = matchOption(UI_FONTS, value);
              if (uiFont) {
                change({ uiFont });
              }
            }}
          >
            {UI_FONTS.map((font) => (
              <ToggleGroupItem
                key={font}
                value={font}
                className={font === "mono" ? "font-mono" : "font-default"}
              >
                {copy.uiFonts[font]}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        )}
      </SettingSection>

      <div className="grid gap-4 border-t border-border pt-6">
        {save.isError && (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">{copy.saveFailed}</p>
            <p className="text-muted-foreground">{errorText(save.error)}</p>
          </Alert>
        )}
        <div className="flex flex-wrap items-center gap-3">
          <Button
            variant="outline"
            onClick={() => {
              reset();
              schedule({ ...DEFAULT_PREFERENCES });
            }}
          >
            <RotateCcw aria-hidden />
            {copy.reset}
          </Button>
          <output className="block text-sm text-muted-foreground">
            {save.isPending ? copy.saving : save.isSuccess ? copy.saved : ""}
          </output>
        </div>
      </div>
    </div>
  );
}
