import { Link } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";
import {
  IMPORT_LIMIT,
  IMPORTED_NAMES,
  useImportPages,
  type ImportResult,
  type PagePlace,
} from "@/api/pages";
import type { WorkspaceSummary } from "@/api/workspaces";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Field } from "@/components/forms/field";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { formatBytes } from "@/lib/format";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";

const copy = strings.pages.transfer;

type ImportDialogProps = {
  workspace: WorkspaceSummary;
  /** Where the pages go. */
  place: PagePlace;
  /** That place, as the title names it: a page's title, a teamspace's name. */
  placeName: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Runs as one of the new pages is opened, such as to close the drawer it was asked from. */
  onNavigate?: (() => void) | undefined;
};

/**
 * Imports a markdown file, or a zip of them, into a place, then says what it
 * made, with the way to each page at the top of it, and which files of a zip
 * it left out. The file is checked here for what the server would refuse
 * before it is sent: its kind, by its name, and its size.
 */
export function ImportDialog({
  workspace,
  place,
  placeName,
  open,
  onOpenChange,
  onNavigate,
}: ImportDialogProps) {
  const importer = useImportPages(workspace.id);
  const [file, setFile] = useState<File | null>(null);
  const [problem, setProblem] = useState<string | undefined>(undefined);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const found = fileProblem(file);
    setProblem(found);
    if (found === undefined && file !== null) {
      importer.mutate({ place, file });
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent closeLabel={copy.close}>
        <DialogTitle>{copy.title(placeName)}</DialogTitle>
        {importer.data ? (
          <Imported
            workspace={workspace}
            result={importer.data}
            onOpen={() => {
              onOpenChange(false);
              onNavigate?.();
            }}
            onDone={() => onOpenChange(false)}
          />
        ) : (
          <>
            <DialogDescription>{copy.lede}</DialogDescription>
            <form noValidate onSubmit={submit} className="grid gap-4">
              <Field label={copy.file} hint={copy.hint} error={problem}>
                {(control) => (
                  <Input
                    {...control}
                    type="file"
                    accept=".md,.markdown,.zip,text/markdown,application/zip"
                    onChange={(event) => {
                      setProblem(undefined);
                      importer.reset();
                      setFile(event.target.files?.[0] ?? null);
                    }}
                    className="py-2 file:mr-3 file:rounded-full file:border file:border-border file:bg-transparent file:px-3 file:text-sm file:font-medium file:text-foreground"
                  />
                )}
              </Field>
              {importer.isError && <ErrorNotice error={importer.error} />}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                  {copy.cancel}
                </Button>
                <Button type="submit" disabled={importer.isPending}>
                  {importer.isPending ? copy.pending : copy.submit}
                </Button>
              </DialogFooter>
            </form>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

/** Why the server would refuse a file, if it would for its kind or size. */
function fileProblem(file: File | null): string | undefined {
  if (file === null) {
    return copy.pick;
  }
  if (!IMPORTED_NAMES.test(file.name)) {
    return copy.wrongKind;
  }
  if (file.size > IMPORT_LIMIT) {
    return copy.tooLarge(formatBytes(file.size));
  }
  return undefined;
}

type ImportedProps = {
  workspace: WorkspaceSummary;
  result: ImportResult;
  onOpen: () => void;
  onDone: () => void;
};

function Imported({ workspace, result, onOpen, onDone }: ImportedProps) {
  return (
    <div className="grid gap-4">
      <Alert live="polite">
        <p className="text-foreground">{copy.imported(result.count)}</p>
      </Alert>
      <ul aria-label={copy.made} className="grid gap-1">
        {result.pages.map((page) => (
          <li key={page.id}>
            <Button asChild variant="ghost" size="sm" className="max-w-full justify-start">
              <Link
                to="/w/$slug/p/$pageId"
                params={{ slug: workspace.slug, pageId: page.id }}
                onClick={onOpen}
              >
                <span className="truncate">{copy.open(titleOf(page))}</span>
              </Link>
            </Button>
          </li>
        ))}
      </ul>
      {result.skipped.length > 0 && (
        <div className="grid gap-1 text-sm">
          <p className="text-muted-foreground">{copy.skipped(result.skipped.length)}</p>
          <ul className="grid gap-0.5">
            {result.skipped.map((file) => (
              <li key={file.path} className="break-words">
                <code className="font-mono text-xs">{file.path}</code>
                <span className="text-muted-foreground"> · {copy.reasons[file.reason]}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      <DialogFooter>
        <Button onClick={onDone}>{copy.done}</Button>
      </DialogFooter>
    </div>
  );
}
