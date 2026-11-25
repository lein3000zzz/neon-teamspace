import { cn } from "cn";
import {
  AlignLeft,
  Calendar,
  CircleChevronDown,
  CircleDot,
  Clock,
  Hash,
  History,
  IdCard,
  Link,
  List,
  SquareCheck,
  Type,
  User,
  UserPen,
  UserPlus,
  type LucideIcon,
} from "lucide-react";
import type { PropertyKind } from "@/api/databases";

const ICONS: Record<PropertyKind, LucideIcon> = {
  title: Type,
  text: AlignLeft,
  number: Hash,
  select: CircleChevronDown,
  multi_select: List,
  status: CircleDot,
  date: Calendar,
  person: User,
  checkbox: SquareCheck,
  url: Link,
  unique_id: IdCard,
  created_time: Clock,
  created_by: UserPlus,
  last_edited_time: History,
  last_edited_by: UserPen,
};

/** A property type's glyph, beside its name. Decorative: the type is said in words where it matters. */
export function PropertyIcon({ kind, className }: { kind: PropertyKind; className?: string }) {
  const Icon = ICONS[kind];
  return <Icon aria-hidden className={cn("size-3.5 shrink-0", className)} />;
}
