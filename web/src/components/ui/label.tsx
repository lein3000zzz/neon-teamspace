import { cn } from "cn";
import { Label as LabelPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

function Label({ className, ...props }: ComponentProps<typeof LabelPrimitive.Root>) {
  return (
    <LabelPrimitive.Root
      data-slot="label"
      className={cn("text-sm font-medium text-foreground select-none", className)}
      {...props}
    />
  );
}

export { Label };
