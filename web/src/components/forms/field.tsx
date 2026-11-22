import { cn } from "cn";
import { CircleAlert } from "lucide-react";
import { useId, type ReactNode } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";

/** What a field hands its control, so the label, hint and error are tied to it. */
export type FieldControl = {
  id: string;
  "aria-describedby"?: string;
  "aria-invalid"?: true;
};

type FieldProps = {
  label: ReactNode;
  hint?: ReactNode;
  /** Shown under the control, which is then marked invalid. */
  error?: string | undefined;
  /** Beside the label, such as a "Forgot your password?" link. */
  aside?: ReactNode;
  className?: string;
  /**
   * Gets the props for the control, and the label's id, for a control that is
   * a group and is named by aria-labelledby rather than by a <label for>.
   */
  children: (control: FieldControl, labelId: string) => ReactNode;
};

/**
 * A label, its control, then the error and the hint. The control is described
 * by both, so a screen reader reads why it is invalid as it lands on it.
 */
export function Field({ label, hint, error, aside, className, children }: FieldProps) {
  const id = useId();
  const errorId = `${id}-error`;
  const hintId = `${id}-hint`;
  // A hint that says what the error says already, such as a password's
  // minimum, is said once.
  const showHint = hint !== undefined && hint !== error;
  const describedBy = [error ? errorId : undefined, showHint ? hintId : undefined]
    .filter((part) => part !== undefined)
    .join(" ");
  return (
    <div className={cn("grid content-start gap-1.5", className)}>
      <div className="flex flex-wrap items-baseline justify-between gap-x-3">
        <Label id={`${id}-label`} htmlFor={id}>
          {label}
        </Label>
        {aside}
      </div>
      {children(
        {
          id,
          ...(describedBy === "" ? {} : { "aria-describedby": describedBy }),
          ...(error ? { "aria-invalid": true as const } : {}),
        },
        `${id}-label`,
      )}
      {error && (
        <p id={errorId} className="flex items-start gap-1.5 text-sm text-bad">
          <CircleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
          {error}
        </p>
      )}
      {showHint && (
        <p id={hintId} className="text-xs text-muted-foreground">
          {hint}
        </p>
      )}
    </div>
  );
}

type CheckboxFieldProps = {
  label: ReactNode;
  hint?: ReactNode;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
};

/** A checkbox whose whole row, label included, takes the press. */
export function CheckboxField({
  label,
  hint,
  checked,
  disabled,
  onCheckedChange,
}: CheckboxFieldProps) {
  const id = useId();
  return (
    <div className="flex min-h-(--tap) items-start gap-3">
      <Checkbox
        id={id}
        checked={checked}
        disabled={disabled ?? false}
        onCheckedChange={(state) => onCheckedChange(state === true)}
        className="mt-[calc((var(--tap)-1.25rem)/2)]"
        {...(hint === undefined ? {} : { "aria-describedby": `${id}-hint` })}
      />
      <div className="grid min-h-(--tap) content-center gap-0.5">
        <Label htmlFor={id} className="cursor-pointer">
          {label}
        </Label>
        {hint !== undefined && (
          <p id={`${id}-hint`} className="text-xs text-muted-foreground">
            {hint}
          </p>
        )}
      </div>
    </div>
  );
}

/** Moves focus to the first field a submit found wrong, by its name in the form. */
export function focusFirstInvalid(form: HTMLFormElement, names: readonly string[]) {
  for (const name of names) {
    const element = form.elements.namedItem(name);
    if (element instanceof HTMLElement) {
      element.focus();
      return;
    }
  }
}
