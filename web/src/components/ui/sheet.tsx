import { cn } from "cn";
import { X } from "lucide-react";
import { Dialog as SheetPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { Button } from "./button";

/**
 * A modal panel that slides in from an edge: from the left, the sidebar on
 * narrow screens; from the right, a row opened beside its database.
 */
function Sheet(props: ComponentProps<typeof SheetPrimitive.Root>) {
  return <SheetPrimitive.Root data-slot="sheet" {...props} />;
}

function SheetTrigger(props: ComponentProps<typeof SheetPrimitive.Trigger>) {
  return <SheetPrimitive.Trigger data-slot="sheet-trigger" {...props} />;
}

function SheetTitle({ className, ...props }: ComponentProps<typeof SheetPrimitive.Title>) {
  return <SheetPrimitive.Title data-slot="sheet-title" className={cn(className)} {...props} />;
}

type SheetContentProps = ComponentProps<typeof SheetPrimitive.Content> & {
  /** Accessible name of the close button. */
  closeLabel: string;
  /** The edge it slides in from. */
  side?: "left" | "right";
};

const sides = {
  left: "inset-y-0 left-0 w-[min(18rem,85vw)] border-r safe-top-3 pr-3 safe-bottom-4 safe-left-3 data-[state=closed]:slide-out-to-left data-[state=open]:slide-in-from-left",
  right:
    "inset-y-0 right-0 w-[min(46rem,92vw)] border-l safe-top-3 safe-right-3 safe-bottom-4 pl-3 data-[state=closed]:slide-out-to-right data-[state=open]:slide-in-from-right",
} as const;

/**
 * Focus is trapped inside while open and returns to the trigger on close;
 * Escape, the close button and a tap on the scrim all dismiss it. The panel
 * pads itself clear of the safe area on the three edges it touches.
 */
function SheetContent({
  className,
  children,
  closeLabel,
  side = "left",
  ...props
}: SheetContentProps) {
  return (
    <SheetPrimitive.Portal>
      <SheetPrimitive.Overlay
        data-slot="sheet-overlay"
        className="fixed inset-0 z-40 bg-[rgb(0_0_0/0.5)] data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0"
      />
      <SheetPrimitive.Content
        data-slot="sheet-content"
        // The title names the panel; there is no separate description.
        aria-describedby={undefined}
        className={cn(
          "fixed z-50 flex flex-col overflow-y-auto overscroll-contain border-border bg-raised data-[state=closed]:animate-out data-[state=open]:animate-in",
          sides[side],
          className,
        )}
        {...props}
      >
        {children}
        <SheetPrimitive.Close asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label={closeLabel}
            className="absolute top-[calc(--spacing(2)+env(safe-area-inset-top,0px))] right-2"
          >
            <X aria-hidden />
          </Button>
        </SheetPrimitive.Close>
      </SheetPrimitive.Content>
    </SheetPrimitive.Portal>
  );
}

export { Sheet, SheetContent, SheetTitle, SheetTrigger };
