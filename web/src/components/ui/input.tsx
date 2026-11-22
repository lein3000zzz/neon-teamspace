import { cn } from "cn";
import type { ComponentProps } from "react";

/**
 * Shared by every text control. A full control height, so it is a 44px target
 * on touch screens; 16px text on phones, which keeps iOS from zooming in on
 * focus; an accent border when focused and a red one while aria-invalid.
 */
export const fieldStyles =
  "w-full min-w-0 rounded-md border border-input bg-background px-3 text-base text-foreground transition-colors placeholder:text-muted-foreground focus-visible:border-primary disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive md:text-sm";

function Input({ className, type = "text", ...props }: ComponentProps<"input">) {
  return (
    <input
      data-slot="input"
      type={type}
      className={cn(fieldStyles, "h-(--tap) read-only:bg-muted", className)}
      {...props}
    />
  );
}

export { Input };
