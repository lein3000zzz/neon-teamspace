import { Monitor, Moon, RotateCcw, Sun } from "lucide-react";
import { SettingSection } from "@/components/settings/setting-section";
import { PageHeader } from "@/components/shell/page-header";
import { AccentPicker } from "@/components/theme/accent-picker";
import { PaletteSwatch } from "@/components/theme/palette-swatch";
import { ThemePreview } from "@/components/theme/theme-preview";
import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { strings } from "@/lib/strings";
import {
  MODE_PREFERENCES,
  PALETTES,
  UI_FONTS,
  matchOption,
  type ModePreference,
} from "@/theme/preferences";
import { useTheme } from "@/theme/theme-provider";

const MODE_ICONS: Record<ModePreference, typeof Monitor> = {
  system: Monitor,
  light: Sun,
  dark: Moon,
};

/**
 * Palette, mode, accent and interface font. Every change applies at once and
 * is remembered on this device.
 */
export function AppearancePage() {
  const { preferences, mode, update, reset } = useTheme();
  const copy = strings.appearance;

  // A single-choice toggle group reports "" when its active item is pressed
  // again; matchOption drops that, so a choice can't be cleared.
  return (
    <>
      <PageHeader eyebrow={copy.eyebrow} title={copy.title} lede={copy.lede} />

      <div className="grid gap-8">
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
                  update({ palette });
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

        <SettingSection title={copy.mode} hint={copy.modeHint(copy.modes[mode])}>
          {(headingId) => (
            <ToggleGroup
              type="single"
              aria-labelledby={headingId}
              value={preferences.mode}
              onValueChange={(value) => {
                const next = matchOption(MODE_PREFERENCES, value);
                if (next) {
                  update({ mode: next });
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
            <AccentPicker
              hue={preferences.accentHue}
              labelledBy={headingId}
              onChange={(accentHue) => update({ accentHue })}
            />
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
                  update({ uiFont });
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

        <div className="border-t border-border pt-6">
          <Button variant="outline" onClick={reset}>
            <RotateCcw aria-hidden />
            {copy.reset}
          </Button>
        </div>
      </div>
    </>
  );
}
