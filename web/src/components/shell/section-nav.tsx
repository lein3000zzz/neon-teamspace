import { Link, type LinkProps } from "@tanstack/react-router";
import type { ReactNode } from "react";

/**
 * For what follows the tabs: their bottom border is the rule above the first
 * part, so that part, or the first part of a page made of several, drops its own.
 */
export const afterSectionNav =
  "[&>:first-child]:border-t-0 [&>:first-child]:pt-0 [&>:first-child>:first-child]:border-t-0 [&>:first-child>:first-child]:pt-0";

export type SectionLink = {
  to: NonNullable<LinkProps["to"]>;
  label: ReactNode;
};

type SectionNavProps = {
  label: string;
  links: readonly SectionLink[];
};

/**
 * The tabs of a page made of several: links, not a tab widget, so each part
 * has its own address. They scroll sideways on a narrow screen rather than
 * wrap, and the current one carries the accent underline.
 */
export function SectionNav({ label, links }: SectionNavProps) {
  return (
    <nav aria-label={label} className="-mx-1 mb-8 overflow-x-auto border-b border-border">
      <ul className="flex min-w-max gap-1 px-1">
        {links.map((link) => (
          <li key={String(link.to)}>
            <Link
              to={link.to}
              activeOptions={{ exact: true }}
              className="relative flex min-h-(--tap) items-center px-3 text-sm whitespace-nowrap text-muted-foreground transition-colors after:absolute after:inset-x-2 after:-bottom-px after:h-0.5 after:rounded-full after:bg-primary after:opacity-0 after:shadow-glow hover:text-foreground aria-[current=page]:text-foreground aria-[current=page]:after:opacity-100"
            >
              {link.label}
            </Link>
          </li>
        ))}
      </ul>
    </nav>
  );
}
