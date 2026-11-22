import { Check, Copy } from "lucide-react";
import { useId, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { strings } from "@/lib/strings";

type CopyFieldProps = {
  label: string;
  value: string;
};

/**
 * A secret or link shown once, in a read-only field with a Copy button. Where
 * the browser has no clipboard (plain http on another host than localhost) or
 * refuses it, the text is selected for copying by hand.
 */
export function CopyField({ label, value }: CopyFieldProps) {
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const copy = strings.copy;

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setState("copied");
    } catch {
      input.current?.select();
      setState("failed");
    }
  };

  return (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <div className="flex gap-2">
        <Input
          ref={input}
          id={id}
          readOnly
          value={value}
          spellCheck={false}
          className="font-mono"
          onFocus={(event) => event.currentTarget.select()}
        />
        <Button type="button" variant="outline" onClick={() => void onCopy()}>
          {state === "copied" ? <Check aria-hidden /> : <Copy aria-hidden />}
          {state === "copied" ? copy.copied : copy.copy}
          <span className="sr-only">{label}</span>
        </Button>
      </div>
      <output className="block text-xs text-muted-foreground empty:hidden">
        {state === "failed" ? copy.failed : ""}
      </output>
    </div>
  );
}
