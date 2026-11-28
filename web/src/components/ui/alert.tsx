import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "cn";
import { CircleAlert, Info } from "lucide-react";
import type { ComponentProps } from "react";

const alertVariants = cva(
  "grid grid-cols-[1rem_minmax(0,1fr)] items-start gap-x-2.5 gap-y-1 rounded-lg border bg-card px-3.5 py-3 text-sm text-foreground",
  {
    variants: {
      tone: {
        info: "border-border",
        error: "border-destructive",
      },
    },
    defaultVariants: { tone: "info" },
  },
);

type AlertProps = Omit<ComponentProps<"div">, "role"> &
  VariantProps<typeof alertVariants> & {
    /**
     * How a screen reader hears it appear: "assertive" (role alert) for what
     * just went wrong, "polite" (role status) for news that can wait its turn,
     * "off" (no role) for a box whose text changes as it is read, such as a
     * log, which a live element inside it announces in short instead.
     */
    live: "assertive" | "polite" | "off";
  };

/**
 * A message in a bordered box. Errors get a red border and their own glyph,
 * so they never rely on colour alone.
 */
function Alert({ className, tone, live, children, ...props }: AlertProps) {
  const Icon = tone === "error" ? CircleAlert : Info;
  return (
    <div
      data-slot="alert"
      role={live === "assertive" ? "alert" : live === "polite" ? "status" : undefined}
      className={cn(alertVariants({ tone }), className)}
      {...props}
    >
      <Icon
        aria-hidden
        className={cn("mt-0.5 size-4", tone === "error" ? "text-bad" : "text-muted-foreground")}
      />
      <div className="grid min-w-0 gap-1">{children}</div>
    </div>
  );
}

function AlertTitle({ className, ...props }: ComponentProps<"p">) {
  return <p data-slot="alert-title" className={cn("font-medium", className)} {...props} />;
}

function AlertDescription({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-description"
      className={cn("grid gap-2 text-muted-foreground", className)}
      {...props}
    />
  );
}

export { Alert, AlertDescription, AlertTitle };
