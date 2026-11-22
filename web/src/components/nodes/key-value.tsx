import { cn } from "cn";
import type { ComponentProps, ReactNode } from "react";

/** Node shape 1: a description list of label/value rows separated by 1px rules. */
export function KeyValueList({ className, ...props }: ComponentProps<"dl">) {
  return <dl className={cn("grid", className)} {...props} />;
}

type KeyValueRowProps = Omit<ComponentProps<"div">, "children"> & {
  label: ReactNode;
  children: ReactNode;
};

/** The label takes just under half the width so values line up down the list. */
export function KeyValueRow({ label, children, className, ...props }: KeyValueRowProps) {
  return (
    <div
      className={cn(
        "grid grid-cols-[minmax(0,46%)_minmax(0,1fr)] gap-4 border-b border-border py-2 last:border-b-0",
        className,
      )}
      {...props}
    >
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-words">{children}</dd>
    </div>
  );
}
