import { cn } from "cn";
import { Slider as SliderPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

type SliderProps = Omit<ComponentProps<typeof SliderPrimitive.Root>, "children"> & {
  /** Accessible name of the thumb, which is the element with role="slider". */
  label: string;
  /** Spoken in place of the bare number, e.g. "250 degrees". */
  valueText?: string;
  /** "spectrum" paints the whole hue wheel on the track and drops the filled range. */
  variant?: "default" | "spectrum";
};

/**
 * A single-thumb slider; arrow keys step, Page Up/Down jump, Home/End go to the
 * ends. The root is a full control height tall, so the whole band around the
 * thin track takes a touch.
 */
function Slider({ className, label, valueText, variant = "default", ...props }: SliderProps) {
  const spectrum = variant === "spectrum";
  return (
    <SliderPrimitive.Root
      data-slot="slider"
      className={cn(
        "relative flex h-(--tap) w-full touch-none items-center select-none data-disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <SliderPrimitive.Track
        data-slot="slider-track"
        className={cn(
          "relative h-2 w-full grow overflow-hidden rounded-full",
          spectrum ? "hue-spectrum" : "bg-muted",
        )}
      >
        {!spectrum && (
          <SliderPrimitive.Range data-slot="slider-range" className="absolute h-full bg-primary" />
        )}
      </SliderPrimitive.Track>
      <SliderPrimitive.Thumb
        data-slot="slider-thumb"
        aria-label={label}
        {...(valueText === undefined ? {} : { "aria-valuetext": valueText })}
        className="block size-5 shrink-0 cursor-grab rounded-full border-2 border-background bg-primary shadow-glow ring-1 ring-primary active:cursor-grabbing"
      />
    </SliderPrimitive.Root>
  );
}

export { Slider };
