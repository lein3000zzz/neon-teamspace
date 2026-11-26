import { useId } from "react";
import { EntityMark } from "@/components/nodes/entity-mark";
import { AccentPicker } from "@/components/theme/accent-picker";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { hueFor } from "@/lib/hue";
import { strings } from "@/lib/strings";

const copy = strings.hue;

type HueControlProps = {
  /** The id its hue is hashed from; unset for one not made yet, whose id the server draws. */
  id?: string | undefined;
  name: string;
  icon?: string | null | undefined;
  /** The hue pinned, or null for the one its id hashes to. */
  value: number | null;
  onChange: (value: number | null) => void;
};

/**
 * Where a workspace's, teamspace's or agent's hue comes from: its id, by
 * default, as every client hashes it alike; or a hue pinned on the slider and
 * presets the accent picker uses. A preview shows the mark with the hue
 * chosen, once there is one: one not made yet has no id to hash.
 */
export function HueControl({ id, name, icon, value, onChange }: HueControlProps) {
  const labelId = useId();
  const pickerId = useId();
  const hashed = id === undefined ? undefined : hueFor(id);
  const shown = value ?? hashed;

  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-center gap-4">
        <span id={labelId} className="sr-only">
          {copy.modeLabel}
        </span>
        <ToggleGroup
          type="single"
          aria-labelledby={labelId}
          value={value === null ? "hashed" : "pinned"}
          onValueChange={(mode) => {
            if (mode === "hashed") {
              onChange(null);
            } else if (mode === "pinned") {
              onChange(value ?? hashed ?? hueFor(name));
            }
          }}
        >
          <ToggleGroupItem value="hashed">{copy.hashed}</ToggleGroupItem>
          <ToggleGroupItem value="pinned">{copy.pinned}</ToggleGroupItem>
        </ToggleGroup>
        {shown !== undefined && (
          <span className="flex min-w-0 items-center gap-2 text-sm">
            <span className="sr-only">{copy.preview}</span>
            <EntityMark name={name} icon={icon} hue={shown} />
            <span className="truncate">{name}</span>
            <span className="font-mono text-xs text-muted-foreground tabular-nums">{shown}°</span>
          </span>
        )}
      </div>
      {value === null ? (
        <p className="text-sm text-muted-foreground">
          {hashed === undefined ? copy.hashedLater : copy.hashedHint(hashed)}
        </p>
      ) : (
        <>
          <span id={pickerId} className="sr-only">
            {copy.slider}
          </span>
          <AccentPicker
            hue={value}
            onChange={onChange}
            labelledBy={pickerId}
            sliderLabel={copy.slider}
          />
        </>
      )}
    </div>
  );
}
