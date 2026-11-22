import { cn } from "cn";
import type { ComponentProps } from "react";
import { fieldStyles } from "./input";

function Textarea({ className, ...props }: ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(fieldStyles, "min-h-24 py-2 leading-normal", className)}
      {...props}
    />
  );
}

export { Textarea };
