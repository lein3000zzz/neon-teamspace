import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "cn";
import { Slot } from "radix-ui";
import type { ComponentProps } from "react";

/**
 * Pill-shaped actions. Only `default` and `pill` carry the accent; the rest
 * stay monochrome so the accent keeps meaning "the thing to press".
 */
const buttonVariants = cva(
  "inline-flex shrink-0 cursor-pointer items-center justify-center gap-1.5 rounded-full border text-sm font-medium whitespace-nowrap transition-colors select-none disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        // Mixing towards the foreground darkens in light mode and lightens in
        // dark, so hover always moves away from the label colour.
        default:
          "border-primary bg-primary text-primary-foreground shadow-glow hover:border-[color-mix(in_oklch,var(--primary),var(--foreground)_14%)] hover:bg-[color-mix(in_oklch,var(--primary),var(--foreground)_14%)]",
        pill: "border-primary bg-transparent text-primary hover:bg-primary hover:text-primary-foreground",
        outline:
          "border-border bg-transparent text-foreground hover:border-primary aria-expanded:border-primary",
        ghost: "border-transparent text-muted-foreground hover:bg-accent hover:text-foreground",
        link: "border-transparent text-primary underline-offset-4 hover:underline",
      },
      // Heights and minimum widths come from the --tap tokens, so every size
      // is at least a 44px square on touch screens, even with a short label.
      size: {
        default: "h-(--tap) min-w-(--tap) px-4",
        sm: "h-(--tap-sm) min-w-(--tap-sm) px-3 text-xs",
        lg: "h-(--tap-lg) min-w-(--tap-lg) px-6 text-base",
        icon: "size-(--tap)",
        "icon-sm": "size-(--tap-sm)",
      },
    },
    // After the size classes: a link sizes to its text, but never below the
    // touch target.
    compoundVariants: [{ variant: "link", className: "h-auto min-h-(--tap) px-0" }],
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

type ButtonProps = ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    /** Render the child element (a router link, say) with the button's styles. */
    asChild?: boolean;
  };

function Button({ className, variant, size, asChild = false, ...props }: ButtonProps) {
  const Comp = asChild ? Slot.Root : "button";
  return (
    <Comp
      data-slot="button"
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  );
}

export { Button };
