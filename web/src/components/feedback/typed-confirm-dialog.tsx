import { useId, useState, type FormEvent, type ReactNode } from "react";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { strings } from "@/lib/strings";

type TypedConfirmDialogProps = {
  open: boolean;
  title: ReactNode;
  description: ReactNode;
  /** What must be typed, exactly: the name of what goes. */
  name: string;
  confirm: string;
  onConfirm: (typed: string) => void;
  onCancel: () => void;
};

/**
 * Asks before something that takes a whole workspace away: the confirming
 * button stays off until the name is typed as it is, so it is never one
 * press away.
 */
export function TypedConfirmDialog({
  open,
  title,
  description,
  name,
  confirm,
  onConfirm,
  onCancel,
}: TypedConfirmDialogProps) {
  const inputId = useId();
  const [typed, setTyped] = useState("");
  const matches = typed === name;

  const close = () => {
    setTyped("");
    onCancel();
  };
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (matches) {
      setTyped("");
      onConfirm(typed);
    }
  };

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          close();
        }
      }}
    >
      <AlertDialogContent>
        <AlertDialogTitle>{title}</AlertDialogTitle>
        <AlertDialogDescription>{description}</AlertDialogDescription>
        <form noValidate onSubmit={submit} className="grid gap-4">
          <div className="grid gap-1.5">
            <Label htmlFor={inputId}>{strings.typedConfirm.label(name)}</Label>
            <Input
              id={inputId}
              autoComplete="off"
              spellCheck={false}
              value={typed}
              onChange={(event) => setTyped(event.target.value)}
            />
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button type="button" variant="outline">
                {strings.confirm.cancel}
              </Button>
            </AlertDialogCancel>
            <Button type="submit" disabled={!matches}>
              {confirm}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  );
}
