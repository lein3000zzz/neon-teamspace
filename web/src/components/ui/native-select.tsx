import { cn } from "cn";
import { ChevronDown } from "lucide-react";
import type { ComponentProps } from "react";
import { fieldStyles } from "./input";

/**
 * The platform's own select, styled like the other fields. Phones open their
 * native picker, which beats any custom list on a small touch screen.
 */
function NativeSelect({ className, ...props }: ComponentProps<"select">) {
  return (
    <span className="relative inline-flex w-full">
      <select
        data-slot="native-select"
        className={cn(fieldStyles, "h-(--tap) cursor-pointer appearance-none pr-9", className)}
        {...props}
      />
      <ChevronDown
        aria-hidden
        className="pointer-events-none absolute top-1/2 right-3 size-4 -translate-y-1/2 text-muted-foreground"
      />
    </span>
  );
}

export { NativeSelect };
