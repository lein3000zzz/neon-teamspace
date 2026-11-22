import { Check } from "lucide-react";
import { hueScope } from "@/components/nodes/hue";
import { Slider } from "@/components/ui/slider";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { strings } from "@/lib/strings";
import { HUE_MAX, isHue } from "@/theme/preferences";
import { ACCENT_PRESETS } from "@/theme/presets";

type AccentPickerProps = {
  hue: number;
  onChange: (hue: number) => void;
  /** Id of the heading that names the control. */
  labelledBy: string;
};

/** A free hue slider over the whole wheel, plus one-tap presets. */
export function AccentPicker({ hue, onChange, labelledBy }: AccentPickerProps) {
  const preset = ACCENT_PRESETS.find((candidate) => candidate.hue === hue);
  return (
    <div className="grid gap-4">
      <div className="flex items-center gap-4">
        <Slider
          variant="spectrum"
          label={strings.appearance.accentSlider}
          valueText={strings.appearance.accentValue(hue)}
          min={0}
          max={HUE_MAX}
          step={1}
          value={[hue]}
          onValueChange={([next]) => {
            if (isHue(next)) {
              onChange(next);
            }
          }}
        />
        <output
          aria-hidden
          className="w-12 shrink-0 text-right font-mono text-sm text-muted-foreground tabular-nums"
        >
          {hue}°
        </output>
      </div>

      <ToggleGroup
        type="single"
        variant="swatches"
        aria-labelledby={labelledBy}
        value={preset === undefined ? "" : String(preset.hue)}
        onValueChange={(value) => {
          const picked = ACCENT_PRESETS.find((candidate) => String(candidate.hue) === value);
          if (picked) {
            onChange(picked.hue);
          }
        }}
      >
        {ACCENT_PRESETS.map((candidate) => (
          <ToggleGroupItem
            key={candidate.name}
            value={String(candidate.hue)}
            aria-label={candidate.name}
            title={candidate.name}
          >
            <span
              {...hueScope(candidate.hue)}
              className="grid size-6 place-items-center rounded-full bg-neon text-primary-foreground ring-offset-2 ring-offset-background group-data-[state=on]:shadow-glow group-data-[state=on]:ring-2 group-data-[state=on]:ring-foreground"
            >
              <Check aria-hidden className="hidden size-3.5 group-data-[state=on]:block" />
            </span>
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
    </div>
  );
}
