import { cn } from "cn";
import { Popover as PopoverPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

/**
 * A panel that opens beside the button that opens it, such as the icon and
 * cover pickers. Escape or a tap outside closes it and gives focus back.
 */
function Popover(props: ComponentProps<typeof PopoverPrimitive.Root>) {
  return <PopoverPrimitive.Root data-slot="popover" {...props} />;
}

function PopoverTrigger(props: ComponentProps<typeof PopoverPrimitive.Trigger>) {
  return <PopoverPrimitive.Trigger data-slot="popover-trigger" {...props} />;
}

function PopoverContent({
  className,
  sideOffset = 6,
  align = "start",
  ...props
}: ComponentProps<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        data-slot="popover-content"
        sideOffset={sideOffset}
        align={align}
        collisionPadding={8}
        className={cn(
          "z-50 grid max-h-(--radix-popover-content-available-height) w-[min(22rem,calc(100vw-1rem))] content-start gap-3 overflow-y-auto overscroll-contain rounded-lg border border-border bg-popover p-3 text-popover-foreground data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0",
          className,
        )}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
}

export { Popover, PopoverContent, PopoverTrigger };
