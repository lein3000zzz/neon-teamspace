import type { ReactNode } from "react";

type PageHeaderProps = {
  eyebrow: ReactNode;
  title: ReactNode;
  lede?: ReactNode;
  children?: ReactNode;
};

/** Every page opens the same way: eyebrow, heading, one positioning paragraph. */
export function PageHeader({ eyebrow, title, lede, children }: PageHeaderProps) {
  return (
    <header className="mb-10 grid gap-3">
      <p className="font-mono text-xs tracking-wider text-muted-foreground">{eyebrow}</p>
      <h1 className="text-3xl font-semibold sm:text-4xl">{title}</h1>
      {lede !== undefined && <p className="max-w-[62ch] text-muted-foreground">{lede}</p>}
      {children}
    </header>
  );
}
