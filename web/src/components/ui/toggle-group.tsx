import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "cn";
import { ToggleGroup as ToggleGroupPrimitive } from "radix-ui";
import { createContext, use, type ComponentProps } from "react";

type Variant = NonNullable<VariantProps<typeof groupVariants>["variant"]>;

const groupVariants = cva("flex w-fit", {
  variants: {
    variant: {
      /** The segmented pill control from lein3000.dev. */
      segmented: "items-center gap-0.5 rounded-full border border-border p-0.5",
      /** Larger choices that hold a preview, laid out in a wrapping row. */
      cards: "flex-wrap gap-3",
      /** Round colour swatches; the item draws its own selected state. */
      swatches: "flex-wrap gap-1",
    },
  },
  defaultVariants: { variant: "segmented" },
});

// The selected item is an accent fill with accent-ink text: the one pairing
// whose contrast is guaranteed at every hue.
const itemVariants = cva(
  "inline-flex cursor-pointer items-center justify-center gap-1.5 text-sm whitespace-nowrap transition-colors select-none disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        segmented:
          "h-(--tap-sm) rounded-full px-3 text-muted-foreground hover:text-foreground data-[state=on]:bg-primary data-[state=on]:text-primary-foreground data-[state=on]:shadow-glow",
        cards:
          "flex-col items-stretch gap-2 rounded-lg border border-border bg-card p-2 text-foreground hover:border-primary data-[state=on]:border-primary data-[state=on]:shadow-glow",
        swatches: "group size-(--tap) rounded-full",
      },
    },
    defaultVariants: { variant: "segmented" },
  },
);

const VariantContext = createContext<Variant>("segmented");

/**
 * A set of mutually exclusive choices (role="radiogroup" for type="single").
 * Arrow keys move between items, Tab leaves the group.
 */
function ToggleGroup({
  className,
  variant = "segmented",
  ...props
}: ComponentProps<typeof ToggleGroupPrimitive.Root> & VariantProps<typeof groupVariants>) {
  const resolved = variant ?? "segmented";
  return (
    <VariantContext value={resolved}>
      <ToggleGroupPrimitive.Root
        data-slot="toggle-group"
        className={cn(groupVariants({ variant: resolved }), className)}
        {...props}
      />
    </VariantContext>
  );
}

function ToggleGroupItem({
  className,
  ...props
}: ComponentProps<typeof ToggleGroupPrimitive.Item>) {
  const variant = use(VariantContext);
  return (
    <ToggleGroupPrimitive.Item
      data-slot="toggle-group-item"
      className={cn(itemVariants({ variant }), className)}
      {...props}
    />
  );
}

export { ToggleGroup, ToggleGroupItem };
